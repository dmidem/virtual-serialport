use std::io::{Read, Write};

use virtual_serialport::VirtualPort;

fn main() {
    let (mut port1, mut port2) = VirtualPort::pair(9600, 1024).unwrap();
    let write_data = b"hello";
    let mut read_data = [0u8; 5];

    port1.write_all(write_data).unwrap();
    port2.read_exact(&mut read_data).unwrap();
    assert_eq!(&read_data, write_data);
}
