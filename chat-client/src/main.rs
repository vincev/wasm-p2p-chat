// Copyright (C) 2022  Vince Vasta
// SPDX-License-Identifier: MIT
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
#![warn(clippy::all, rust_2018_idioms, unused_crate_dependencies)]
use async_channel as channel;
use async_executor::Executor;
use futures::prelude::*;
use libp2p::{
    core::{upgrade::Version, Transport},
    floodsub::{self, Floodsub, FloodsubEvent},
    identity, mplex,
    multiaddr::{Multiaddr, Protocol},
    noise,
    swarm::{keep_alive, SwarmBuilder, SwarmEvent},
    NetworkBehaviour, PeerId,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use websys_transport::WebsocketTransport;

use std::{collections::VecDeque, time::Duration};

static TASK_EXECUTOR: Executor<'_> = Executor::new();

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

// Wasm build.
fn main() {
    // Make sure panics are logged using `console.error`.
    console_error_panic_hook::set_once();

    let web_options = eframe::WebOptions::default();
    eframe::start_web(
        "chat_canvas",
        web_options,
        Box::new(|cc| Box::new(MainApp::new(cc))),
    )
    .expect("failed to start eframe");
}

pub struct MainApp {
    message_rx: channel::Receiver<String>,
    command_tx: channel::Sender<Command>,
    messages: VecDeque<String>,
}

impl MainApp {
    /// Create a new main egui app instance.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let (message_tx, message_rx) = channel::bounded(64);
        let (command_tx, command_rx) = channel::bounded(64);

        network_service(command_rx, message_tx);

        Self {
            message_rx,
            command_tx,
            messages: Default::default(),
        }
    }
}

impl eframe::App for MainApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(message) = self.message_rx.try_recv() {
            self.messages.push_back(message);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for m in &self.messages {
                        ui.label(&format!("Message {m}"));
                    }
                    ui.allocate_space(ui.available_size());
                });
        });

        // Run 10 frames per second.
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

enum Command {
    Dial(Multiaddr),
    Chat(String),
}

fn network_service(
    mut command_rx: channel::Receiver<Command>,
    message_tx: channel::Sender<String>,
) {
    // Create the websocket transport.
    let local_key = identity::Keypair::generate_ed25519();
    let transport = WebsocketTransport::default()
        .upgrade(Version::V1)
        .authenticate(noise::NoiseAuthenticated::xx(&local_key).unwrap())
        .multiplex(mplex::MplexConfig::default())
        .boxed();

    // Create a behaviour to receive Floodsub messages and keep alive connection.
    #[derive(NetworkBehaviour)]
    struct Behaviour {
        keep_alive: keep_alive::Behaviour,
        floodsub: Floodsub,
    }

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let local_peer_id = PeerId::from(local_key.public());
        let mut behaviour = Behaviour {
            floodsub: Floodsub::new(local_peer_id),
            keep_alive: keep_alive::Behaviour::default(),
        };

        let floodsub_topic = floodsub::Topic::new("chat");
        behaviour.floodsub.subscribe(floodsub_topic);

        SwarmBuilder::new(transport, behaviour, local_peer_id)
            .executor(Box::new(|fut| {
                TASK_EXECUTOR.spawn(fut).detach();
            }))
            .build()
    };

    // Starts the executor.
    spawn_local(TASK_EXECUTOR.run(futures::future::pending::<()>()));

    // Spawn task to manage Swarm events and UI channels.
    TASK_EXECUTOR
        .spawn(async move {
            loop {
                futures::select! {
                    command = command_rx.select_next_some() => match command {
                        Command::Dial(_addr) => {}
                        Command::Chat(_message) => {}
                    },
                    event = swarm.select_next_some() => match event {
                        SwarmEvent::Behaviour(BehaviourEvent::Floodsub(FloodsubEvent::Message(message))) => {
                            let _ = message_tx.send(String::from_utf8_lossy(&message.data).to_string()).await;
                        },
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            let _ = message_tx.send(format!("Connected to {peer_id}")).await;
                        }
                        SwarmEvent::ConnectionClosed { peer_id, .. } => {
                            let _ = message_tx.send(format!("Connection close to {peer_id}")).await;
                        }
                        _ => {}
                    }
                }
            }
        })
        .detach();

    console_log!("Started executor");
}
