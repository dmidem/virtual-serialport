# Virtual Serial Port

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

## Example Usage

### Loopback Example

```rust
use std::io::{Read, Write};

use virtual_serialport::VirtualPort;

let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();
let write_data = b"hello";
let mut read_data = [0u8; 5];

port.write(write_data).unwrap();
port.read(&mut read_data).unwrap();
assert_eq!(&read_data, write_data);
```

### Pair Example
```
use std::io::{Read, Write};

use virtual_serialport::VirtualPort;

let (mut port1, mut port2) = VirtualPort::open_pair(9600, 1024).unwrap();
let write_data = b"hello";
let mut read_data = [0u8; 5];

port1.write(write_data).unwrap();
port2.read(&mut read_data).unwrap();
assert_eq!(&read_data, write_data);
```

More examples can be found in the `examples` folder in the root of this
repository.
