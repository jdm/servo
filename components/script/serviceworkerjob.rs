/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

//! A Job is an abstraction of async operation in service worker lifecycle propagation.
//! Each Job is uniquely identified by its scope_url, and is keyed accordingly under
//! the script thread. The script thread contains a JobQueue, which stores all scheduled Jobs
//! by multiple service worker clients in a Vec.

use dom::bindings::cell::DOMRefCell;
use dom::bindings::error::Error;
use dom::bindings::js::JS;
use dom::bindings::refcounted::{Trusted, TrustedPromise};
use dom::bindings::reflector::DomObject;
use dom::client::Client;
use dom::globalscope::GlobalScope;
use dom::promise::Promise;
use dom::serviceworkerregistration::ServiceWorkerRegistration;
use dom::urlhelper::UrlHelper;
use js::jsapi::JSAutoCompartment;
use script_thread::{ScriptThread, Runnable};
use servo_url::ServoUrl;
use std::cmp::PartialEq;
use std::collections::HashMap;
use std::rc::Rc;
use task_source::TaskSource;
use task_source::dom_manipulation::DOMManipulationTaskSource;

#[derive(PartialEq, Copy, Clone, Debug, JSTraceable)]
pub enum JobType {
    Register,
    Unregister,
    Update
}

#[derive(Clone)]
pub enum SettleType {
    Resolve(Trusted<ServiceWorkerRegistration>),
    Reject(Error)
}

#[derive(PartialEq)]
enum FinishJob {
    Invoke,
    DoNotInvoke,
}

#[must_root]
#[derive(JSTraceable)]
pub struct Job {
    pub job_type: JobType,
    pub scope_url: ServoUrl,
    pub script_url: ServoUrl,
    pub promise: Rc<Promise>,
    pub equivalent_jobs: Vec<Job>,
    // client can be a window client, worker client so `Client` will be an enum in future
    pub client: JS<Client>,
    pub referrer: ServoUrl
}

impl Job {
    #[allow(unrooted_must_root)]
    // https://w3c.github.io/ServiceWorker/#create-job-algorithm
    pub fn create_job(job_type: JobType,
                      scope_url: ServoUrl,
                      script_url: ServoUrl,
                      promise: Rc<Promise>,
                      client: &Client) -> Job {
        Job {
            job_type: job_type,
            scope_url: scope_url,
            script_url: script_url,
            promise: promise,
            equivalent_jobs: vec![],
            client: JS::from_ref(client),
            referrer: client.creation_url()
        }
    }
    #[allow(unrooted_must_root)]
    pub fn append_equivalent_job(&mut self, job: Job) {
        self.equivalent_jobs.push(job);
    }
}

impl PartialEq for Job {
    // Equality criteria as described in https://w3c.github.io/ServiceWorker/#dfn-job-equivalent
    fn eq(&self, other: &Self) -> bool {
        let same_job = self.job_type == other.job_type;
        if same_job {
            match self.job_type {
                JobType::Register | JobType::Update => {
                    self.scope_url == other.scope_url && self.script_url == other.script_url
                },
                JobType::Unregister => self.scope_url == other.scope_url
            }
        } else {
            false
        }
    }
}

pub struct AsyncJobHandler {
    pub scope_url: ServoUrl,
}

impl AsyncJobHandler {
    fn new(scope_url: ServoUrl) -> AsyncJobHandler {
        AsyncJobHandler {
            scope_url: scope_url,
        }
    }
}

impl Runnable for AsyncJobHandler {
    #[allow(unrooted_must_root)]
    fn main_thread_handler(self: Box<AsyncJobHandler>, script_thread: &ScriptThread) {
        script_thread.dispatch_job_queue(self);
    }
}

#[must_root]
#[derive(JSTraceable)]
pub struct JobQueue(pub DOMRefCell<HashMap<ServoUrl, Vec<Job>>>);

impl JobQueue {
    pub fn new() -> JobQueue {
        JobQueue(DOMRefCell::new(HashMap::new()))
    }
    #[allow(unrooted_must_root)]
    // https://w3c.github.io/ServiceWorker/#schedule-job-algorithm
    pub fn schedule_job(&self,
                        job: Job,
                        global: &GlobalScope,
                        script_thread: &ScriptThread) {
        let mut queue_ref = self.0.borrow_mut();
        let job_queue = queue_ref.entry(job.scope_url.clone()).or_insert(vec![]);
        // Step 1
        if job_queue.is_empty() {
            let scope_url = job.scope_url.clone();
            job_queue.push(job);
            let run_job_handler = box AsyncJobHandler::new(scope_url);
            let _ = script_thread.dom_manipulation_task_source().queue(run_job_handler, global);
        } else {
            // Step 2
            let mut last_job = job_queue.pop().unwrap();
            if job == last_job && !last_job.promise.is_settled() {
                last_job.append_equivalent_job(job);
                job_queue.push(last_job);
            } else {
                // restore the popped last_job
                job_queue.push(last_job);
                // and push this new job to job queue
                job_queue.push(job);
            }
        }
    }

    #[allow(unrooted_must_root)]
    // https://w3c.github.io/ServiceWorker/#run-job-algorithm
    pub fn run_job(&self, run_job_handler: Box<AsyncJobHandler>, script_thread: &ScriptThread) {
        let (finish_job, url) = {
            let queue_ref = self.0.borrow();
            let front_job = {
                let job_vec = queue_ref.get(&run_job_handler.scope_url);
                job_vec.unwrap().first().unwrap()
            };
            let script_url = front_job.script_url.clone();
            (match front_job.job_type {
                JobType::Register => self.run_register(front_job, run_job_handler, script_thread),
                JobType::Update => self.update(front_job, script_thread),
                JobType::Unregister => unreachable!(),
            }, script_url)
        };
        if finish_job == FinishJob::Invoke {
            self.finish_job(url, script_thread);
        }
    }

    #[allow(unrooted_must_root)]
    // https://w3c.github.io/ServiceWorker/#register-algorithm
    pub fn run_register(&self, job: &Job, register_job_handler: Box<AsyncJobHandler>, script_thread: &ScriptThread) -> FinishJob {
        let AsyncJobHandler { scope_url, .. } = *register_job_handler;
        // Step 1-3
        if !UrlHelper::is_origin_trustworthy(&job.script_url) {
            // Step 1.1
            reject_job_promise(job,
                               Error::Type("Invalid script ServoURL".to_owned()),
                               script_thread.dom_manipulation_task_source());
            // Step 1.2
            return FinishJob::Invoke;
        } else if job.script_url.origin() != job.referrer.origin() || job.scope_url.origin() != job.referrer.origin() {
            // Step 2.1/3.1
            reject_job_promise(job,
                               Error::Security,
                               script_thread.dom_manipulation_task_source());
            // Step 2.2/3.2
            return FinishJob::Invoke;
        }
        
        // Step 4-5
        if let Some(reg) = script_thread.handle_get_registration(&job.scope_url) {
            // Step 5.1
            if reg.get_uninstalling() {
                reg.set_uninstalling(false);
            }
            // Step 5.3
            if let Some(ref newest_worker) = reg.get_newest_worker() {
                if (&*newest_worker).get_script_url() == job.script_url {
                    // Step 5.3.1
                    resolve_job_promise(job, &*reg, script_thread.dom_manipulation_task_source());
                    // Step 5.3.2
                    return FinishJob::Invoke;
                }
            }
        } else {
            // Step 6.1
            let global = &*job.client.global();
            let pipeline = global.pipeline_id();
            let new_reg = ServiceWorkerRegistration::new(&*global, &job.script_url, scope_url);
            script_thread.handle_serviceworker_registration(&job.scope_url, &*new_reg, pipeline);
        }
        // Step 7
        self.update(job, script_thread)
    }

    #[allow(unrooted_must_root)]
    // https://w3c.github.io/ServiceWorker/#finish-job-algorithm
    pub fn finish_job(&self, scope_url: ServoUrl, script_thread: &ScriptThread) {
        let run_job = if let Some(job_vec) = (*self.0.borrow_mut()).get_mut(&scope_url) {
            if job_vec.first().map_or(false, |job| job.scope_url == scope_url) {
                let _  = job_vec.remove(0);
            }
            !job_vec.is_empty()
        } else {
            warn!("non-existent job vector for Servourl: {:?}", scope_url);
            false
        };

        if run_job {
            let handler = box AsyncJobHandler::new(scope_url);
            self.run_job(handler, script_thread);
        }
    }

    // https://w3c.github.io/ServiceWorker/#update-algorithm
    pub fn update(&self, job: &Job, script_thread: &ScriptThread) -> FinishJob {
        // Step 1
        let reg = match script_thread.handle_get_registration(&job.scope_url) {
            Some(reg) => reg,
            None => {
                let err_type = Error::Type("No registration to update".to_owned());
                // Step 2.1
                reject_job_promise(job, err_type, script_thread.dom_manipulation_task_source());
                // Step 2.2
                return FinishJob::Invoke;
            }
        };
        // Step 2
        if reg.get_uninstalling() {
            let err_type = Error::Type("Update called on an uninstalling registration".to_owned());
            // Step 2.1
            reject_job_promise(job, err_type, script_thread.dom_manipulation_task_source());
            // Step 2.2
            return FinishJob::Invoke;
        }
        // Step 3
        let newest_worker = match reg.get_newest_worker() {
            Some(worker) => worker,
            None => return FinishJob::DoNotInvoke,
        };
        // Step 4
        if (&*newest_worker).get_script_url() == job.script_url  && job.job_type == JobType::Update {
            let err_type = Error::Type("Invalid script ServoURL".to_owned());
            // Step 4.1
            reject_job_promise(job, err_type, script_thread.dom_manipulation_task_source());
            // Step 4.2
            return FinishJob::Invoke;
        }
        // Step 8
        job.client.set_controller(&*newest_worker);
        // Step 8.1
        resolve_job_promise(job, &*reg, script_thread.dom_manipulation_task_source());
        // Step 8.2
        FinishJob::Invoke
    }
}

struct AsyncPromiseSettle {
    global: Trusted<GlobalScope>,
    promise: TrustedPromise,
    settle_type: SettleType,
}

impl Runnable for AsyncPromiseSettle {
    #[allow(unrooted_must_root)]
    fn handler(self: Box<AsyncPromiseSettle>) {
        let global = self.global.root();
        let settle_type = self.settle_type.clone();
        let promise = self.promise.root();
        settle_job_promise(&*global, &*promise, settle_type)
    }
}

impl AsyncPromiseSettle {
    #[allow(unrooted_must_root)]
    fn new(promise: Rc<Promise>, settle_type: SettleType) -> AsyncPromiseSettle {
        AsyncPromiseSettle {
            global: Trusted::new(&*promise.global()),
            promise: TrustedPromise::new(promise),
            settle_type: settle_type,
        }
    }
}

fn settle_job_promise(global: &GlobalScope, promise: &Promise, settle: SettleType) {
    let _ac = JSAutoCompartment::new(global.get_cx(), promise.reflector().get_jsobject().get());
    match settle {
        SettleType::Resolve(reg) => promise.resolve_native(global.get_cx(), &*reg.root()),
        SettleType::Reject(err) => promise.reject_error(global.get_cx(), err),
    };
}

// https://w3c.github.io/ServiceWorker/#reject-job-promise-algorithm
// https://w3c.github.io/ServiceWorker/#resolve-job-promise-algorithm
fn queue_settle_job_promise(job: &Job, settle: SettleType, task_source: &DOMManipulationTaskSource) {
    let task = box AsyncPromiseSettle::new(job.promise.clone(), settle);
    let global = job.client.global();
    // Step 1
    let _ = task_source.queue(task, &*global);

    // TODO step 2 - queue tasks for equivalent job promises
}

fn reject_job_promise(job: &Job, err: Error, task_source: &DOMManipulationTaskSource) {
    queue_settle_job_promise(job, SettleType::Reject(err), task_source)
}

fn resolve_job_promise(job: &Job, reg: &ServiceWorkerRegistration, task_source: &DOMManipulationTaskSource) {
    queue_settle_job_promise(job, SettleType::Resolve(Trusted::new(reg)), task_source)
}
