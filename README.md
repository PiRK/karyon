# karyons

An infrastructure for peer-to-peer, decentralized, and collaborative software.

> In molecular biology, a Karyon is essentially "a part of the cell
> containing DNA and RNA and responsible for growth and reproduction"

Join us on:

- [Discord](https://discord.gg/xuXRcrkz3p)

## Crates 

- [karyons core](./core):  Essential utilities and core functionality.
- [karyons net](./net): Provides a network interface for TCP, UDP, and Unix,
  along with common network functionality. 
- [karyons p2p](./p2p): A lightweight, extensible, and customizable
  peer-to-peer (p2p) network stack.
- [karyons jsonrpc](./jsonrpc): A fast and lightweight async
  [JSONRPC2.0](https://www.jsonrpc.org/specification) implementation.
- karyons crdt: A [CRDT](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type) 
implementation for building collaborative software. 
- karyons base: A lightweight, extensible database that operates with karyons crdt.

## Status

This project is a work in progress. The current focus is on shipping karyons
crdt and karyons store, along with major changes to the network stack,
including TLS implementation. You can check the issues for updates on ongoing
tasks.

## Contribution

Feel free to open a pull request or an issue. 

## License

All the code in this repository is licensed under the GNU General Public
License, version 3 (GPL-3.0). You can find a copy of the license in the
[LICENSE](./LICENSE) file.

