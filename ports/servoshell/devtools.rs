/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;

use crossbeam_channel::{unbounded, Sender, Receiver, TryRecvError};
use serde::Serialize;

pub(crate) enum DevtoolCommand {
    Close,
}

#[derive(Debug)]
pub(crate) struct ConsoleLog {
    pub message: String,
    pub level: Level,
    pub filename: String,
    pub column: u32,
    pub line_number: u32,
}

#[derive(Debug)]
pub(crate) enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug)]
pub(crate) enum DevtoolUpdate {
    ConsoleLog(ConsoleLog),
    Navigate,
}

pub(crate) struct DevtoolClient {
    sender: Sender<DevtoolCommand>,
    receiver: Receiver<DevtoolUpdate>,
}

impl DevtoolClient {
    pub(crate) fn receive_updates(&self) -> Result<Vec<DevtoolUpdate>, ()> {
        let mut updates = vec![];
        loop {
            match self.receiver.try_recv() {
                Ok(update) => updates.push(update),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Err(()),
            }
        }
        Ok(updates)
    }
}

impl Drop for DevtoolClient {
    fn drop(&mut self) {
        _ = self.sender.send(DevtoolCommand::Close);
    }
}

fn do_connect(port: u16) -> Result<TcpStream, ()> {
    let stream = TcpStream::connect(&format!("127.0.0.1:{port}"))
        .map_err(|e| { println!("Error connecting to devtools server: {e:?}"); () })?;
    println!("connected to server");
    Ok(stream)
}

fn read_packet(stream: &mut TcpStream) -> Result<Vec<u8>, ()> {
    let mut digits = vec![];
    loop {
        let mut byte = [0];
        let num_read = stream.read(&mut byte).map_err(|_| ())?;
        if num_read <= 0 {
            return Err(());
        }
        if byte[0] == b':' {
            let digits = str::from_utf8(&digits).unwrap();
            let size: usize = digits.parse().unwrap();
            let mut packet = vec![0; size];
            let num_read = stream.read(packet.as_mut_slice()).map_err(|_| ())?;
            if num_read <= 0 {
                return Err(());
            }
            return Ok(packet);
        } else {
            digits.push(byte[0]);
        }
    }
}

fn handle_root_packet(packet: &serde_json::Map<String, serde_json::Value>, stream: &mut TcpStream, _sender: &Sender<DevtoolUpdate>) {
    if packet.contains_key("applicationType") {
        send_packet(ConnectPacket {
            type_: "connect".to_string(),
            frontend_version: "144.0".to_string(),
            to: "root".to_string(),
        }, stream);
        send_packet(GetRootPacket {
            type_: "getRoot".to_string(),
            to: "root".to_string(),
        }, stream);
        send_packet(ListTabsPacket {
            type_: "listTabs".to_string(),
            to: "root".to_string(),
        }, stream);
        send_packet(GetTabPacket {
            type_: "getTab".to_string(),
            browser_id: 1,
            to: "root".to_string(),
        }, stream);
        send_packet(GetCachedMessagesPacket {
            type_: "getCachedMessages".to_string(),
            message_types: vec!["LogMessage".to_string(), "PageError".to_string(), "ConsoleAPI".to_string()],
            to: "console4".to_string(),
        }, stream);
    }
}

fn handle_console_packet(packet: &serde_json::Map<String, serde_json::Value>, _stream: &mut TcpStream, _sender: &Sender<DevtoolUpdate>) {
    let Some(messages) = packet.get("messages") else {
        return;
    };
    let messages = messages.as_array().unwrap();
    for message in messages {
        println!("{:?}", message);
    }
}

fn handle_target_packet(packet: &serde_json::Map<String, serde_json::Value>, _stream: &mut TcpStream, sender: &Sender<DevtoolUpdate>) {
    if packet.get("type").is_none_or(|t| t.as_str().unwrap() != "resources-available-array") {
        return;
    }
    let Some(messages) = packet.get("array") else {
        return;
    };
    let messages = messages.as_array().unwrap();
    for message in messages {
        let message = message.as_array().unwrap();
        match message[0].as_str().unwrap() {
            "console-message" => {
                let messages = message[1].as_array().unwrap();
                for message in messages {
                    let message = message.as_object().unwrap();
                    let level = message.get("level").unwrap().as_str().unwrap();
                    let level = match level {
                        "warn" => Level::Warn,
                        "error" => Level::Error,
                        _ => Level::Info,
                    };
                    let filename = message.get("filename").unwrap().as_str().unwrap();
                    let line_number = message.get("line_number").unwrap().as_u64().unwrap();
                    let column_number = message.get("column_number").unwrap().as_u64().unwrap();
                    let message = message.get("arguments").unwrap().as_array().unwrap()[0].as_str().unwrap();
                    let data = ConsoleLog {
                        filename: filename.to_string(),
                        line_number: line_number as u32,
                        column: column_number as u32,
                        message: message.to_string(),
                        level,
                    };
                    let _ = sender.send(DevtoolUpdate::ConsoleLog(data));
                }
            }
            "error-message" => {
                let messages = message[1].as_array().unwrap();
                for message in messages {
                    let message = message.as_object().unwrap();
                    let message = message.get("pageError").unwrap().as_object().unwrap();
                    let level = if message.get("warning").unwrap().as_bool().unwrap() { Level::Warn } else { Level::Error };
                    let filename = message.get("sourceName").unwrap().as_str().unwrap();
                    let line_number = message.get("lineNumber").unwrap().as_u64().unwrap();
                    let column_number = message.get("columnNumber").unwrap().as_u64().unwrap();
                    let message = message.get("errorMessage").unwrap().as_str().unwrap();
                    let data = ConsoleLog {
                        filename: filename.to_string(),
                        line_number: line_number as u32,
                        column: column_number as u32,
                        message: message.to_string(),
                        level,
                    };
                    let _ = sender.send(DevtoolUpdate::ConsoleLog(data));
                }
            }
            _ => {
                println!("ignoring unknown message type");
            }
        }
        println!("{:?}", message);
    }

}

fn handle_packet(packet: &serde_json::Map<String, serde_json::Value>, stream: &mut TcpStream, sender: &Sender<DevtoolUpdate>) {
    // {"from":"root","applicationType":"browser","traits":{"sources":false,"highlightable":true,"customHighlighters":true,"networkMonitor":true}}

    //{"from":"target5","type":"resources-available-array","array":[["console-message",[{"level":"log","filename":"file:///tmp/devtool.html","line_number":2,"column_number":8,"time_stamp":1761377897891,"arguments":["hello there"]}]]]}
    // {"from":"target5","type":"resources-available-array","array":[["error-message",[{"pageError":{"_type":"PageError","errorMessage":"a is not defined","sourceName":"file:///tmp/devtool.html","lineText":"","lineNumber":2,"columnNumber":1,"category":"script","timeStamp":1761377911857,"error":true,"warning":false,"exception":true,"strict":false,"private":false}}]]]}
    let from = packet.get("from").unwrap().as_str().unwrap();
    if from == "root" {
        handle_root_packet(packet, stream, sender);
        return;
    }
    if from.starts_with("console") {
        handle_console_packet(packet, stream, sender);
        return;
    }
    if from.starts_with("target") {
        handle_target_packet(packet, stream, sender);
        return;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectPacket {
    #[serde(rename = "type")]
    type_: String,
    frontend_version: String,
    to: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetRootPacket {
    #[serde(rename = "type")]
    type_: String,
    to: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListTabsPacket {
    #[serde(rename = "type")]
    type_: String,
    to: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetTabPacket {
    #[serde(rename = "type")]
    type_: String,
    browser_id: u32,
    to: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GetCachedMessagesPacket {
    #[serde(rename = "type")]
    type_: String,
    message_types: Vec<String>,
    to: String,
}

fn send_packet<T: Serialize>(packet: T, stream: &mut TcpStream) {
    fn send_packet_inner(packet: String, stream: &mut TcpStream) {
        let size = packet.len();
        let complete = format!("{}:{}", size.to_string(), packet);
        let num_written = stream.write(complete.as_bytes()).unwrap();
        assert_eq!(complete.len(), num_written);
    }
    send_packet_inner(serde_json::to_string(&packet).unwrap(), stream)
}

pub(crate) fn connect(port: u16) -> Receiver<Result<DevtoolClient, ()>> {
    println!("trying to connect");
    let (sender, receiver) = unbounded();
    thread::spawn(move || {
        let stream = match do_connect(port) {
            Ok(stream) => stream,
            Err(e) => {
                let _ = sender.send(Err(e));
                return;
            }
        };

        let (command_sender, command_receiver) = unbounded();
        let (update_sender, update_receiver) = unbounded();
        let mut stream2 = stream.try_clone().unwrap();
        let command_sender2 = command_sender.clone();

        thread::spawn(move || {
            loop {
                match read_packet(&mut stream2) {
                    Ok(packet) => {
                        let packet = str::from_utf8(&packet).unwrap();
                        println!("{}", packet);
                        let packet: serde_json::Value = serde_json::from_str(packet).unwrap();
                        handle_packet(packet.as_object().unwrap(), &mut stream2, &update_sender);
                    }
                    Err(_e) => {
                        let _ = command_sender2.send(DevtoolCommand::Close);
                        return;
                    }
                }
            }
        });

        let client = DevtoolClient {
            sender: command_sender,
            receiver: update_receiver,
        };
        let _ = sender.send(Ok(client));

        loop {
            match command_receiver.recv() {
                Ok(DevtoolCommand::Close) => {
                    return;
                }
                Err(e) => {
                    println!("Terminating devtool client due to {e:?}");
                    return;
                }
            }
        }
    });
    receiver
}
