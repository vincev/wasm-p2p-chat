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
use egui::{Color32, RichText};
use futures::prelude::*;
use libp2p::{
    core::{upgrade::Version, Transport},
    floodsub::{self, Floodsub, FloodsubEvent},
    identity, mplex,
    multiaddr::Multiaddr,
    noise,
    swarm::{keep_alive, SwarmBuilder, SwarmEvent},
    NetworkBehaviour, PeerId,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use websys_transport::WebsocketTransport;

use std::{collections::VecDeque, time::Duration};

// Debugging console log.
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
    event_rx: channel::Receiver<Event>,
    command_tx: channel::Sender<Command>,
    messages: VecDeque<(Color32, String)>,
    connected: bool,
    text: String,
}

impl MainApp {
    /// Create a new main egui app instance.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let (event_tx, event_rx) = channel::bounded(64);
        let (command_tx, command_rx) = channel::bounded(64);

        // Start libp2p network service.
        spawn_local(network_service(command_rx, event_tx));

        Self {
            event_rx,
            command_tx,
            messages: Default::default(),
            connected: false,
            text: "/ip4/127.0.0.1/tcp/9876/ws".to_string(),
        }
    }

    fn send_command(&self, command: Command) {
        let tx = self.command_tx.clone();
        spawn_local(async move {
            let _ = tx.send(command).await;
        });
    }

    fn send_chat(&mut self) {
        self.send_command(Command::Chat(self.text.clone()));
        self.messages
            .push_back((Color32::LIGHT_BLUE, format!("{: >20}", self.text)));
        self.text.clear();
    }
}

impl eframe::App for MainApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process events coming from the network service.
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                Event::Message(text) => {
                    self.messages.push_back((Color32::GREEN, text));
                }
                Event::Connected(peer_id) => {
                    self.connected = true;
                    self.text.clear();
                    self.messages
                        .push_back((Color32::YELLOW, format!("Connected to {peer_id}")));
                }
                Event::Disconnected(peer_id) => {
                    self.connected = true;
                    self.messages
                        .push_back((Color32::YELLOW, format!("Disconnected from {peer_id}")));
                }
                Event::Error(e) => {
                    self.connected = false;
                    self.messages
                        .push_back((Color32::RED, format!("Error: {e}")))
                }
            }
        }

        // Render commands panel.
        egui::TopBottomPanel::bottom("command").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.connected {
                    if ui.button("Chat").clicked() {
                        self.send_chat();
                    }
                } else if ui.button("Connect").clicked() {
                    if let Ok(address) = self.text.parse::<Multiaddr>() {
                        self.send_command(Command::Dial(address));
                    } else {
                        self.messages.push_back((
                            Color32::RED,
                            format!("Invalid multiaddr {}", self.text.clone()),
                        ));
                    }
                }
                let r = ui.text_edit_singleline(&mut self.text);
                if r.lost_focus() && ui.input().key_pressed(egui::Key::Enter) {
                    self.send_chat();
                }
            });
        });

        // Render message panel.
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (color, text) in &self.messages {
                        ui.label(RichText::new(text).size(14.0).monospace().color(*color));
                    }
                    ui.allocate_space(ui.available_size());
                });
        });

        // Run 20 frames per second.
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

enum Command {
    Dial(Multiaddr),
    Chat(String),
}

enum Event {
    Connected(PeerId),
    Disconnected(PeerId),
    Message(String),
    Error(String),
}

async fn network_service(
    mut command_rx: channel::Receiver<Command>,
    event_tx: channel::Sender<Event>,
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

    let floodsub_topic = floodsub::Topic::new("chat");

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let local_peer_id = PeerId::from(local_key.public());
        let mut behaviour = Behaviour {
            floodsub: Floodsub::new(local_peer_id),
            keep_alive: keep_alive::Behaviour::default(),
        };

        behaviour.floodsub.subscribe(floodsub_topic.clone());

        SwarmBuilder::new(transport, behaviour, local_peer_id)
            .executor(Box::new(|fut| {
                spawn_local(fut);
            }))
            .build()
    };

    // Manage Swarm events and UI channels.
    loop {
        futures::select! {
            command = command_rx.select_next_some() => match command {
                Command::Dial(addr) => {
                    if let Err(e) = swarm.dial(addr) {
                        let _ = event_tx.send(Event::Error(e.to_string())).await;
                    }
                }
                Command::Chat(message) => {
                    swarm
                        .behaviour_mut()
                        .floodsub
                        .publish(floodsub_topic.clone(), message.as_bytes());
                }
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(BehaviourEvent::Floodsub(FloodsubEvent::Message(message))) => {
                    let event = Event::Message(String::from_utf8_lossy(&message.data).into());
                    let _ = event_tx.send(event).await;
                },
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    swarm
                        .behaviour_mut()
                        .floodsub
                        .add_node_to_partial_view(peer_id);
                    let _ = event_tx.send(Event::Connected(peer_id)).await;
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    swarm
                        .behaviour_mut()
                        .floodsub
                        .remove_node_from_partial_view(&peer_id);
                    let _ = event_tx.send(Event::Disconnected(peer_id)).await;
                }
                SwarmEvent::OutgoingConnectionError { error, .. } => {
                    let _ = event_tx.send(Event::Error(error.to_string())).await;
                }
                event => console_log!("Swarm event: {event:?}"),
            }
        }
    }
}
