# Virtual Serial Port

[![Crates.io][crates-badge]][crates]
[![Docs.rs][docs-badge]][docs]
[![Actions][actions-badge]][actions]
[![MSRV][msrv-badge]][msrv]
[![Release][release-badge]][release]
[![License][license-badge]][license]

[crates-badge]: https://img.shields.io/crates/v/virtual-serialport.svg
[crates]: https://crates.io/crates/virtual-serialport
[docs-badge]: https://docs.rs/virtual-serialport/badge.svg
[docs]: https://docs.rs/virtual-serialport
[actions-badge]: https://github.com/dmidem/virtual-serialport/workflows/CI/badge.svg
[actions]: https://github.com/dmidem/virtual-serialport/actions/workflows/ci.yml=branch%3Amain
[msrv-badge]: https://img.shields.io/crates/msrv/virtual-serialport.svg
[msrv]: https://github.com/dmidem/virtual-serialport/Cargo.toml
[release-badge]: https://img.shields.io/github/v/release/dmidem/virtual-serialport.svg
[release]: https://github.com/dmidem/virtual-serialport/releases/latest
[license-badge]: https://img.shields.io/crates/l/virtual-serialport.svg
[license]: #license

The Serial Port Simulator (virtual port) is designed to work alongside the
[`serialport`](https://crates.io/crates/serialport) crate. It supports
reading from and writing to the port using internal buffers, with optional
timeout functionality.

The simulator also allows configuring standard serial port parameters, such as:

- baud rate
- data bits
- parity
- stop bits
- flow control

[Documentation](https://docs.rs/virtual-serialport)

Additional features include:

- **Control Signal Simulation**: Simulates control signals (RTS/CTS,
  DTR/DSR/CD). Note that actual flow control based on these signals is not
  implemented.

- **Transmission Delay Simulation**: When enabled, simulates transmission delay
  based on the baud rate. This is implemented in a simplified manner by adding
  a fixed delay for each symbol read (the delay is calculated according to the
  baud rate).

- **Noise Simulation**: If enabled, simulates noise when the physical settings
  (baud rate, data bits, parity, and stop bits) of paired ports do not match.
  This helps test how the system handles corrupted or invalid data under
  mismatched configurations.

## Example

```rust
use std::io::{Read, Write};

use virtual_serialport::VirtualPort;

let (mut port1, mut port2) = VirtualPort::pair(9600, 1024).unwrap();
let write_data = b"hello";
let mut read_data = [0u8; 5];

port1.write_all(write_data).unwrap();
port2.read_exact(&mut read_data).unwrap();
assert_eq!(&read_data, write_data);
```

More examples can be found in the [examples](examples) folder in the root of this repository.

## License

Licensed under either of Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or MIT license ([LICENSE-MIT](LICENSE-MIT)) at your option.
