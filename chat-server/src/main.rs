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

use clap::Parser;
use futures::prelude::*;
use libp2p::{
    core::{upgrade::Version, Transport},
    floodsub::{self, Floodsub, FloodsubEvent},
    identity, mplex,
    multiaddr::{Multiaddr, Protocol},
    noise,
    swarm::{keep_alive, NetworkBehaviour, SwarmEvent},
    tcp, websocket, PeerId, Swarm,
};

use std::{error::Error, net::Ipv4Addr};

#[derive(Debug, Parser)]
struct Cli {
    /// Listen for connection on this port.
    #[clap(long, default_value_t = 9876)]
    port: u16,
}

#[async_std::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    // Create a random PeerId
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    println!("Local peer id: {:?}", local_peer_id);

    let transport = websocket::WsConfig::new(tcp::async_io::Transport::new(tcp::Config::new()))
        .upgrade(Version::V1)
        .authenticate(noise::NoiseAuthenticated::xx(&local_key)?)
        .multiplex(mplex::MplexConfig::default())
        .boxed();

    #[derive(NetworkBehaviour)]
    struct Behaviour {
        keep_alive: keep_alive::Behaviour,
        floodsub: Floodsub,
    }

    let floodsub_topic = floodsub::Topic::new("chat");
    // Create a Swarm to manage peers and events
    let mut swarm = {
        let mut behaviour = Behaviour {
            floodsub: Floodsub::new(local_peer_id),
            keep_alive: keep_alive::Behaviour::default(),
        };

        behaviour.floodsub.subscribe(floodsub_topic.clone());
        Swarm::with_async_std_executor(transport, behaviour, local_peer_id)
    };

    // Listen for connections on the given port.
    let address = Multiaddr::from(Ipv4Addr::UNSPECIFIED)
        .with(Protocol::Tcp(cli.port))
        .with(Protocol::Ws("/".into()));
    swarm.listen_on(address)?;

    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("Listening on {:?}", address);
            }
            SwarmEvent::Behaviour(BehaviourEvent::Floodsub(FloodsubEvent::Message(message))) => {
                println!(
                    "Received: '{:?}' from {:?}",
                    String::from_utf8_lossy(&message.data),
                    message.source
                );
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                println!("ConnectionEstablished to: {peer_id}");
                swarm
                    .behaviour_mut()
                    .floodsub
                    .add_node_to_partial_view(peer_id);
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                println!("ConnectionClosed to: {peer_id}");
                swarm
                    .behaviour_mut()
                    .floodsub
                    .remove_node_from_partial_view(&peer_id);
            }
            event => println!("SwarmEvent {event:?}"),
        }
    }
}
