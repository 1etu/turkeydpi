use std::net::SocketAddr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use backend::{BypassProxy, ProxyConfig};
use engine::BypassConfig;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::tray::PROXY_FAILED;

pub enum Command {
    Start {
        bypass: BypassConfig,
        listen: SocketAddr,
        notify: isize,
    },
    Stop,
    Quit,
}

pub struct ProxyThread {
    tx: Sender<Command>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ProxyThread {
    pub fn spawn() -> Self {
        let (tx, rx) = channel();
        let handle = thread::spawn(move || worker(rx));

        Self {
            tx,
            handle: Some(handle),
        }
    }

    pub fn start(&self, bypass: BypassConfig, listen: SocketAddr, notify: isize) {
        let _ = self.tx.send(Command::Start {
            bypass,
            listen,
            notify,
        });
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker(rx: Receiver<Command>) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return,
    };

    let mut running: Option<tokio::task::JoinHandle<()>> = None;

    while let Ok(command) = rx.recv() {
        match command {
            Command::Start {
                bypass,
                listen,
                notify,
            } => {
                if let Some(task) = running.take() {
                    task.abort();
                }

                let config = ProxyConfig {
                    listen_addr: listen,
                    bypass,
                    quiet: true,
                    ..Default::default()
                };

                running = Some(runtime.spawn(async move {
                    let mut proxy = BypassProxy::new(config);
                    if let Err(e) = proxy.run().await {
                        tracing::error!("proxy stopped: {}", e);
                        unsafe {
                            PostMessageW(notify as HWND, PROXY_FAILED, 0, 0);
                        }
                    }
                }));
            }
            Command::Stop => {
                if let Some(task) = running.take() {
                    task.abort();
                }
            }
            Command::Quit => {
                if let Some(task) = running.take() {
                    task.abort();
                }
                break;
            }
        }
    }

    runtime.shutdown_background();
}
