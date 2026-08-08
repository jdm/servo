/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Memory profiling functions.

use std::borrow::ToOwned;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs::File;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use log::debug;
use parking_lot::{Condvar, Mutex};
use profile_traits::mem::{
    MemoryReport, MemoryReportResult, ProfilerChan, ProfilerMsg, Report, ReportKind, Reporter, ReporterRequest,
    ReportsChan,
};
use profile_traits::path;
use servo_base::generic_channel::{self, GenericCallback, GenericReceiver};

use crate::system_reporter;

const LOG_FILE_VAR: &str = "UNTRACKED_LOG_FILE";

pub struct Profiler {
    /// The port through which messages are received.
    pub port: GenericReceiver<ProfilerMsg>,

    /// Registered memory reporters.
    reporters: HashMap<String, Reporter>,

    /// Instant at which this profiler was created.
    created: Instant,

    /// Used to notify the timer thread that the profiler is done
    /// with processing a `ProfilerMsg::Print` message.
    notifier: Arc<(Mutex<bool>, Condvar)>,
}

const JEMALLOC_HEAP_ALLOCATED_STR: &str = "jemalloc-heap-allocated";
const SYSTEM_HEAP_ALLOCATED_STR: &str = "system-heap-allocated";

impl Profiler {
    pub fn create(period: Option<u16>) -> ProfilerChan {
        let (chan, port) = generic_channel::channel().unwrap();

        if servo_allocator::is_tracking_unmeasured() && std::env::var(LOG_FILE_VAR).is_err() {
            eprintln!("Allocation tracking is enabled but {LOG_FILE_VAR} is unset.");
        }

        let notifier = Arc::new((Mutex::new(false), Condvar::new()));
        let notifier2 = Arc::clone(&notifier);

        // Create the timer thread if a period was provided.
        if let Some(period) = period {
            let chan = chan.clone();
            thread::Builder::new()
                .name("MemoryProfTimer".to_owned())
                .spawn(move || {
                    loop {
                        thread::sleep(Duration::from_secs(period.into()));
                        let (mutex, cvar) = &*notifier;
                        let mut done = mutex.lock();
                        *done = false;
                        if chan.send(ProfilerMsg::Print).is_err() {
                            break;
                        }
                        if !*done {
                            cvar.wait(&mut done);
                        }
                    }
                })
                .expect("Thread spawning failed");
        }

        // Always spawn the memory profiler. If there is no timer thread it won't receive regular
        // `Print` events, but it will still receive the other events.
        thread::Builder::new()
            .name("MemoryProfiler".to_owned())
            .spawn(move || {
                let mut mem_profiler = Profiler::new(port, notifier2);
                mem_profiler.start();
            })
            .expect("Thread spawning failed");

        let mem_profiler_chan = ProfilerChan(chan);

        // Register the system memory reporter, which will run on its own thread. It never needs to
        // be unregistered, because as long as the memory profiler is running the system memory
        // reporter can make measurements.
        let callback = GenericCallback::new(|message| {
            let request: ReporterRequest = message.unwrap();
            system_reporter::collect_reports(request)
        })
        .expect("Could not create system reporter callback");
        mem_profiler_chan.send(ProfilerMsg::RegisterReporter(
            "system-main".to_owned(),
            Reporter(callback),
        ));

        mem_profiler_chan
    }

    pub fn new(port: GenericReceiver<ProfilerMsg>, notifier: Arc<(Mutex<bool>, Condvar)>) -> Profiler {
        Profiler {
            port,
            reporters: HashMap::new(),
            created: Instant::now(),
            notifier,
        }
    }

    pub fn start(&mut self) {
        while let Ok(msg) = self.port.recv() {
            if !self.handle_msg(msg) {
                break;
            }
        }
    }

    fn handle_msg(&mut self, msg: ProfilerMsg) -> bool {
        match msg {
            ProfilerMsg::RegisterReporter(name, reporter) => {
                debug!("Registering memory reporter: {}", name);
                // Panic if it has already been registered.
                let name_clone = name.clone();
                match self.reporters.insert(name, reporter) {
                    None => true,
                    Some(_) => panic!("RegisterReporter: '{}' name is already in use", name_clone),
                }
            },

            ProfilerMsg::UnregisterReporter(name) => {
                debug!("Unregistering memory reporter: {}", name);
                // Panic if it hasn't previously been registered.
                match self.reporters.remove(&name) {
                    Some(_) => true,
                    None => panic!("UnregisterReporter: '{}' name is unknown", &name),
                }
            },

            ProfilerMsg::Report(sender) => {
                let main_pid = std::process::id();

                let reports = self.collect_reports();
                // Turn the pid -> reports map into a vector and add the
                // hint to find the main process.
                let results: Vec<MemoryReport> = reports
                    .into_iter()
                    .map(|(pid, reports)| MemoryReport {
                        pid,
                        reports,
                        is_main_process: pid == main_pid,
                    })
                    .collect();
                let _ = sender.send(MemoryReportResult { results });

                if let Ok(value) = std::env::var(LOG_FILE_VAR) {
                    match File::create(&value) {
                        Ok(file) => {
                            servo_allocator::dump_unmeasured(file);
                        },
                        Err(error) => {
                            log::error!("Error creating log file: {error:?}");
                        },
                    }
                }

                // Notify the timer thread.
                let (mutex, cvar) = &*self.notifier;
                let mut done = mutex.lock();
                *done = true;
                cvar.notify_one();

                true
            },

            ProfilerMsg::Print => {
                self.handle_print_msg();
                // Notify the timer thread.
                let (mutex, cvar) = &*self.notifier;
                let mut done = mutex.lock();
                *done = true;
                cvar.notify_one();
                true
            },

            ProfilerMsg::Exit => false,
        }
    }

    /// Returns a map of pid -> reports
    fn collect_reports(&self) -> HashMap<u32, Vec<Report>> {
        let mut result = HashMap::new();

        for reporter in self.reporters.values() {
            let (chan, port) = generic_channel::channel().unwrap();
            reporter.collect_reports(ReportsChan(chan));
            if let Ok(mut reports) = port.recv() {
                result
                    .entry(reports.pid)
                    .or_insert(vec![])
                    .append(&mut reports.reports);
            }
        }
        result
    }

    fn handle_print_msg(&self) {
        let elapsed = self.created.elapsed();
        println!("Begin memory reports {}", elapsed.as_secs());
        println!("|");

        // Collect reports from memory reporters.
        //
        // This serializes the report-gathering. It might be worth creating a new scoped thread for
        // each reporter once we have enough of them.
        //
        // If anything goes wrong with a reporter, we just skip it.
        //
        // We also track the total memory reported on the jemalloc heap and the system heap, and
        // use that to compute the special "jemalloc-heap-unclassified" and
        // "system-heap-unclassified" values.

        let mut forest = ReportsForest::new();

        let mut jemalloc_heap_reported_size = 0;
        let mut system_heap_reported_size = 0;

        let mut jemalloc_heap_allocated_size: Option<usize> = None;
        let mut system_heap_allocated_size: Option<usize> = None;

        for reporter in self.reporters.values() {
            let (chan, port) = generic_channel::channel().unwrap();
            reporter.collect_reports(ReportsChan(chan));
            if let Ok(mut reports) = port.recv() {
                for report in &mut reports.reports {
                    println!("{:?}", report);
                    // Add "explicit" to the start of the path, when appropriate.
                    match report.kind {
                        ReportKind::ExplicitJemallocHeapSize |
                        ReportKind::ExplicitSystemHeapSize |
                        ReportKind::ExplicitNonHeapSize |
                        ReportKind::ExplicitUnknownLocationSize => {
                            report.path.insert(0, String::from("explicit"))
                        },
                        ReportKind::NonExplicitSize => {},
                    }

                    // Update the reported fractions of the heaps, when appropriate.
                    match report.kind {
                        ReportKind::ExplicitJemallocHeapSize => {
                            jemalloc_heap_reported_size += report.size
                        },
                        ReportKind::ExplicitSystemHeapSize => {
                            system_heap_reported_size += report.size
                        },
                        _ => {},
                    }

                    // Record total size of the heaps, when we see them.
                    if report.path.len() == 1 {
                        if report.path[0] == JEMALLOC_HEAP_ALLOCATED_STR {
                            assert!(jemalloc_heap_allocated_size.is_none());
                            jemalloc_heap_allocated_size = Some(report.size);
                        } else if report.path[0] == SYSTEM_HEAP_ALLOCATED_STR {
                            assert!(system_heap_allocated_size.is_none());
                            system_heap_allocated_size = Some(report.size);
                        }
                    }

                    // Insert the report.
                    forest.insert(&report.path, report.size);
                }
            }
        }

        // Compute and insert the heap-unclassified values.
        if let Some(jemalloc_heap_allocated_size) = jemalloc_heap_allocated_size {
            forest.insert(
                &path!["explicit", "jemalloc-heap-unclassified"],
                jemalloc_heap_allocated_size - jemalloc_heap_reported_size,
            );
        }
        if let Some(system_heap_allocated_size) = system_heap_allocated_size {
            forest.insert(
                &path!["explicit", "system-heap-unclassified"],
                system_heap_allocated_size - system_heap_reported_size,
            );
        }

        forest.print();

        println!("|");
        println!("End memory reports");
        println!();
    }
}

/// A collection of one or more reports with the same initial path segment. A ReportsTree
/// containing a single node is described as "degenerate".
struct ReportsTree {
    /// For leaf nodes, this is the sum of the sizes of all reports that mapped to this location.
    /// For interior nodes, this is the sum of the sizes of all its child nodes.
    size: usize,

    /// For leaf nodes, this is the count of all reports that mapped to this location.
    /// For interor nodes, this is always zero.
    count: u32,

    /// The segment from the report path that maps to this node.
    path_seg: String,

    /// Child nodes.
    children: Vec<ReportsTree>,
}

impl ReportsTree {
    fn new(path_seg: String) -> ReportsTree {
        ReportsTree {
            size: 0,
            count: 0,
            path_seg,
            children: vec![],
        }
    }

    // Searches the tree's children for a path_seg match, and returns the index if there is a
    // match.
    fn find_child(&self, path_seg: &str) -> Option<usize> {
        for (i, child) in self.children.iter().enumerate() {
            if child.path_seg == *path_seg {
                return Some(i);
            }
        }
        None
    }

    // Insert the path and size into the tree, adding any nodes as necessary.
    fn insert(&mut self, path: &[String], size: usize) {
        let mut t: &mut ReportsTree = self;
        for path_seg in path {
            let i = match t.find_child(path_seg) {
                Some(i) => i,
                None => {
                    let new_t = ReportsTree::new(path_seg.clone());
                    t.children.push(new_t);
                    t.children.len() - 1
                },
            };
            let tmp = t; // this temporary is needed to satisfy the borrow checker
            t = &mut tmp.children[i];
        }

        t.size += size;
        t.count += 1;
    }

    // Fill in sizes for interior nodes and sort sub-trees accordingly. Should only be done once
    // all the reports have been inserted.
    fn compute_interior_node_sizes_and_sort(&mut self) -> usize {
        if !self.children.is_empty() {
            // Interior node. Derive its size from its children.
            if self.size != 0 {
                // This will occur if e.g. we have paths ["a", "b"] and ["a", "b", "c"].
                panic!("one report's path is a sub-path of another report's path");
            }
            for child in &mut self.children {
                self.size += child.compute_interior_node_sizes_and_sort();
            }
            // Now that child sizes have been computed, we can sort the children.
            self.children.sort_by(|t1, t2| t2.size.cmp(&t1.size));
        }
        self.size
    }

    fn print(&self, depth: i32) {
        if !self.children.is_empty() {
            assert_eq!(self.count, 0);
        }

        let mut indent_str = String::new();
        for _ in 0..depth {
            indent_str.push_str("   ");
        }

        let mebi = 1024f64 * 1024f64;
        let count_str = if self.count > 1 {
            format!(" [{}]", self.count)
        } else {
            "".to_owned()
        };
        println!(
            "|{}{:8.2} MiB -- {}{}",
            indent_str,
            (self.size as f64) / mebi,
            self.path_seg,
            count_str
        );

        for child in &self.children {
            child.print(depth + 1);
        }
    }
}

/// A collection of ReportsTrees. It represents the data from multiple memory reports in a form
/// that's good to print.
struct ReportsForest {
    trees: HashMap<String, ReportsTree>,
}

impl ReportsForest {
    fn new() -> ReportsForest {
        ReportsForest {
            trees: HashMap::new(),
        }
    }

    // Insert the path and size into the forest, adding any trees and nodes as necessary.
    fn insert(&mut self, path: &[String], size: usize) {
        let (head, tail) = path.split_first().unwrap();
        // Get the right tree, creating it if necessary.
        if !self.trees.contains_key(head) {
            self.trees
                .insert(head.clone(), ReportsTree::new(head.clone()));
        }
        let t = self.trees.get_mut(head).unwrap();

        // Use tail because the 0th path segment was used to find the right tree in the forest.
        t.insert(tail, size);
    }

    fn print(&mut self) {
        // Fill in sizes of interior nodes, and recursively sort the sub-trees.
        for tree in self.trees.values_mut() {
            tree.compute_interior_node_sizes_and_sort();
        }

        // Put the trees into a sorted vector. Primary sort: degenerate trees (those containing a
        // single node) come after non-degenerate trees. Secondary sort: alphabetical order of the
        // root node's path_seg.
        let mut v = vec![];
        for tree in self.trees.values() {
            v.push(tree);
        }
        v.sort_by(|a, b| {
            if a.children.is_empty() && !b.children.is_empty() {
                Ordering::Greater
            } else if !a.children.is_empty() && b.children.is_empty() {
                Ordering::Less
            } else {
                a.path_seg.cmp(&b.path_seg)
            }
        });

        // Print the forest.
        for tree in &v {
            tree.print(0);
            // Print a blank line after non-degenerate trees.
            if !tree.children.is_empty() {
                println!("|");
            }
        }
    }
}
