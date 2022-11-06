# Wasm p2p chat example

An example [libp2p][rust-libp2p] chat app with WASM chat clients running in the browser
and a floodsub server that distributes messages to all clients. All communication is
authenticated and encrypted by [libp2p][rust-libp2p] noise protocol support.

[rust-libp2p]: https://github.com/libp2p/rust-libp2p

<p align="center">
  <img src="/media/clients.png">
</p>

## Build and run

To run the example first run the server:

```shell
$ cd chat-server
$ cargo r
    Finished dev [unoptimized + debuginfo] target(s) in 0.21s
     Running `wasm-p2p-chat/target/debug/chat-server`
Local peer id: PeerId("12D3KooWDvqnZSJ7ZkUTLmr1A2qGBKt5wi11gArpANDZWf8Pn7bX")
Listening on "/ip4/127.0.0.1/tcp/9876/ws"
Listening on "/ip4/192.168.178.94/tcp/9876/ws"
```

Then to build and run the client first install the [`trunk`](https://trunkrs.dev/) build
tool, this can be installed with cargo:

```shell
$ cargo install --locked trunk
```

then build the client:

```shell
$ cd chat-client
$ trunk serve
2022-11-05T16:10:52.451764Z  INFO 📦 starting build
2022-11-05T16:10:52.452724Z  INFO spawning asset pipelines
2022-11-05T16:10:52.680841Z  INFO building chat-client
2022-11-05T16:10:52.680886Z  INFO copying & hashing css path="assets/chat.css"
2022-11-05T16:10:52.681052Z  INFO finished copying & hashing css path="assets/chat.css"
    Finished dev [unoptimized + debuginfo] target(s) in 0.12s
2022-11-05T16:10:52.832222Z  INFO fetching cargo artifacts
2022-11-05T16:10:53.103629Z  INFO processing WASM for chat-client
2022-11-05T16:10:53.150764Z  INFO using system installed binary app=wasm-bindgen version=0.2.83
2022-11-05T16:10:53.150829Z  INFO calling wasm-bindgen for chat-client
2022-11-05T16:10:53.596512Z  INFO copying generated wasm-bindgen artifacts
2022-11-05T16:10:53.597666Z  INFO applying new distribution
2022-11-05T16:10:53.598325Z  INFO ✅ success
2022-11-05T16:10:53.598638Z  INFO 📡 serving static assets at -> /
2022-11-05T16:10:53.598767Z  INFO 📡 server listening at http://127.0.0.1:8080
```

This will build the WASM distribution package and run a server that allows the browser to
load the WASM application by connecting to the address listed in the `trunk` build
output:

<img src="/media/single-client.png">

then press the connect button to connect to the `Multiaddr` in the box, if all is well you
should see the connected message:

<img src="/media/single-connected.png">

Then with multiple clients connected you can send chat messages by typing the message in
the text field and press return or the chat button.





