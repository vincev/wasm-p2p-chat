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

use futures::{future::Ready, io, prelude::*};
use libp2p::{
    core::transport::{ListenerId, Transport, TransportError, TransportEvent},
    multiaddr::{Multiaddr, Protocol},
};
use parking_lot::Mutex;
use send_wrapper::SendWrapper;
use wasm_bindgen::{prelude::*, JsCast};
use web_sys::{ErrorEvent, MessageEvent, WebSocket};

use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    task::Poll,
    task::{Context, Waker},
};

#[derive(Default)]
pub struct WebsocketTransport;

impl Transport for WebsocketTransport {
    type Output = Connection;
    type Error = Error;
    type ListenerUpgrade = Ready<Result<Self::Output, Self::Error>>;
    type Dial = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;

    fn listen_on(&mut self, _addr: Multiaddr) -> Result<ListenerId, TransportError<Self::Error>> {
        Err(TransportError::Other(Error::NotSupported))
    }

    fn remove_listener(&mut self, _id: ListenerId) -> bool {
        false
    }

    fn dial(&mut self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        let ws_url = if let Some(url) = websocket_url(addr) {
            url
        } else {
            return Err(TransportError::Other(Error::NotSupported));
        };

        Ok(async move {
            let socket = match WebSocket::new(&ws_url) {
                Ok(ws) => ws,
                Err(err) => return Err(Error::JsError(format!("{err:?}"))),
            };

            Ok(Connection::new(socket))
        }
        .boxed())
    }

    fn dial_as_listener(
        &mut self,
        _addr: Multiaddr,
    ) -> Result<Self::Dial, TransportError<Self::Error>> {
        Err(TransportError::Other(Error::NotSupported))
    }

    fn poll(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> std::task::Poll<TransportEvent<Self::ListenerUpgrade, Self::Error>> {
        Poll::Pending
    }

    fn address_translation(&self, _listen: &Multiaddr, _observed: &Multiaddr) -> Option<Multiaddr> {
        None
    }
}

// Try to convert Multiaddr to a Websocket url.
fn websocket_url(addr: Multiaddr) -> Option<String> {
    let mut protocols = addr.iter();
    let host_port = match (protocols.next(), protocols.next()) {
        (Some(Protocol::Ip4(ip)), Some(Protocol::Tcp(port))) => {
            format!("{ip}:{port}")
        }
        (Some(Protocol::Ip6(ip)), Some(Protocol::Tcp(port))) => {
            format!("[{ip}]:{port}")
        }
        (Some(Protocol::Dns(h)), Some(Protocol::Tcp(port)))
        | (Some(Protocol::Dns4(h)), Some(Protocol::Tcp(port)))
        | (Some(Protocol::Dns6(h)), Some(Protocol::Tcp(port)))
        | (Some(Protocol::Dnsaddr(h)), Some(Protocol::Tcp(port))) => {
            format!("{}:{}", &h, port)
        }
        _ => return None,
    };

    let (scheme, wspath) = match protocols.next() {
        Some(Protocol::Ws(path)) => ("ws", path.into_owned()),
        Some(Protocol::Wss(path)) => ("wss", path.into_owned()),
        _ => return None,
    };

    // TODO: handle PeerId
    Some(format!("{scheme}://{host_port}{wspath}"))
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("js function error {0}")]
    JsError(String),
    #[error("operation not supported")]
    NotSupported,
}

pub struct Connection {
    shared: Arc<Mutex<Shared>>,
}

struct Shared {
    opened: bool,
    error: Option<String>,
    data: VecDeque<u8>,
    waker: Option<Waker>,
    socket: SendWrapper<WebSocket>,
}

impl Connection {
    fn new(socket: WebSocket) -> Self {
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let shared = Arc::new(Mutex::new(Shared {
            opened: false,
            error: None,
            data: VecDeque::with_capacity(1 << 16),
            waker: None,
            socket: SendWrapper::new(socket.clone()),
        }));

        let onmessage_callback = Closure::<dyn FnMut(_)>::new({
            let shared = shared.clone();
            move |e: MessageEvent| {
                if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let mut locked = shared.lock();
                    let bytes = js_sys::Uint8Array::new(&abuf).to_vec();
                    locked.data.extend(bytes.into_iter());
                    if let Some(waker) = &locked.waker {
                        waker.wake_by_ref();
                    }
                } else {
                    panic!("Unexpected data format {:?}", e.data());
                }
            }
        });
        socket.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();

        let onerror_callback = Closure::<dyn FnMut(_)>::new({
            let shared = shared.clone();
            move |e: ErrorEvent| {
                shared.lock().error = Some(e.message());
            }
        });
        socket.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        onerror_callback.forget();

        let onopen_callback = Closure::<dyn FnMut()>::new({
            let shared = shared.clone();
            move || {
                let mut locked = shared.lock();
                locked.opened = true;
                if let Some(waker) = &locked.waker {
                    waker.wake_by_ref();
                }
            }
        });
        socket.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));
        onopen_callback.forget();

        Self { shared }
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<Result<usize, io::Error>> {
        let mut shared = self.shared.lock();
        shared.waker = Some(cx.waker().clone());

        if let Some(error) = shared.error.as_ref().cloned() {
            Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, error)))
        } else if shared.data.is_empty() {
            Poll::Pending
        } else {
            let n = shared.data.len().min(buf.len());
            for k in buf.iter_mut().take(n) {
                *k = shared.data.pop_front().unwrap();
            }
            Poll::Ready(Ok(n))
        }
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        let mut shared = self.shared.lock();
        shared.waker = Some(cx.waker().clone());

        if let Some(error) = shared.error.as_ref().cloned() {
            Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, error)))
        } else if !shared.opened {
            Poll::Pending
        } else {
            match shared.socket.send_with_u8_array(buf) {
                Ok(()) => Poll::Ready(Ok(buf.len())),
                Err(err) => Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Write error: {err:?}"),
                ))),
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Poll::Pending
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let shared = self.shared.lock();
        let _ = shared.socket.close();
    }
}
