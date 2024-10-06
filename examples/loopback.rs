use std::io::{Read, Write};

use virtual_serialport::VirtualPort;

fn main() {
    let mut port = VirtualPort::loopback(9600, 1024).unwrap();
    let write_data = b"hello";
    let mut read_data = [0u8; 5];

    port.write_all(write_data).unwrap();
    port.read_exact(&mut read_data).unwrap();
    assert_eq!(&read_data, write_data);
}
