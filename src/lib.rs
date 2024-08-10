// FIXME: tests for control lines in pair (stop transmission if disabled?)

//! # Virtual Serial Port
//!
//! The Serial Port Simulator (virtual port) is designed to work alongside the
//! [`serialport`](https://crates.io/crates/serialport) crate. It supports
//! reading from and writing to the port using internal buffers, with optional
//! timeout functionality.
//!
//! The simulator also allows configuring standard serial port parameters, such as:
//!
//! - baud rate
//! - data bits
//! - parity
//! - stop bits
//! - flow control
//!
//! Additional features include:
//!
//! - **Control Signal Simulation**: Simulates control signals (RTS/CTS,
//!   DTR/DSR/CD). Note that actual flow control based on these signals is not
//!   implemented.
//!
//! - **Transmission Delay Simulation**: When enabled, simulates transmission delay
//!   based on the baud rate. This is implemented in a simplified manner by adding
//!   a fixed delay for each symbol read (the delay is calculated according to the
//!   baud rate).
//!
//! - **Noise Simulation**: If enabled, simulates noise when the physical settings
//!   (baud rate, data bits, parity, and stop bits) of paired ports do not match.
//!   This helps test how the system handles corrupted or invalid data under
//!   mismatched configurations.
//!
//! ## Example Usage
//!
//! ### Loopback Example
//! ```
//! use std::io::{Read, Write};
//!
//! use virtual_serialport::VirtualPort;
//!
//! let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();
//! let write_data = b"hello";
//! let mut read_data = [0u8; 5];
//!
//! port.write(write_data).unwrap();
//! port.read(&mut read_data).unwrap();
//! assert_eq!(&read_data, write_data);
//! ```
//!
//! ### Pair Example
//! ```
//! use std::io::{Read, Write};
//!
//! use virtual_serialport::VirtualPort;
//!
//! let (mut port1, mut port2) = VirtualPort::open_pair(9600, 1024).unwrap();
//! let write_data = b"hello";
//! let mut read_data = [0u8; 5];
//!
//! port1.write(write_data).unwrap();
//! port2.read(&mut read_data).unwrap();
//! assert_eq!(&read_data, write_data);
//! ```

// To run doc tests on examples from README.md and verify their correctness
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadMe;

use std::{
    collections::vec_deque::VecDeque,
    io,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use rand::Rng;

use serialport::{
    ClearBuffer, DataBits, Error, ErrorKind, FlowControl, Parity, Result, SerialPort, StopBits,
};

struct Config {
    // Baud rate in symbols per second
    baud_rate: u32,

    // Number of bits per character
    data_bits: DataBits,

    // Flow control mode
    flow_control: FlowControl,

    // Parity checking mode
    parity: Parity,

    // Number of stop bits
    stop_bits: StopBits,

    // Timeout duration for read/write operations
    timeout: Duration,

    // Whether to simulate the delay of data transmission based on baud rate.
    // If enabled, this will add a fixed delay for each symbol read to simulate
    // the transmission delay. Note that this is a simplified simulation: in a real
    // serial communication, transmission would continue even when read operations
    // are not performed, allowing some data to be available immediately when
    // a read is executed. This simulation does not account for such behavior and
    // only introduces a delay per symbol as if transmission was paused during reads
    simulate_delay: bool,

    // Whether to simulate corrupted symbols if physical settings don't match
    noise_on_config_mismatch: bool,
}

impl Config {
    fn new(baud_rate: u32) -> Self {
        Self {
            baud_rate,
            data_bits: DataBits::Eight,
            flow_control: FlowControl::None,
            parity: Parity::None,
            stop_bits: StopBits::One,
            timeout: Duration::from_millis(0),
            simulate_delay: false,
            noise_on_config_mismatch: false,
        }
    }

    // Calculates the total number of bits per byte based on the current configuration.
    // This includes:
    // - 1 start bit (always present)
    // - `data_bits` (5 to 8 data bits depending on configuration)
    // - Optional parity bit (1 bit if parity is `Odd` or `Even`, 0 bits if `None`)
    // - `stop_bits` (1 or 2 bits depending on configuration)
    fn bits_per_byte(&self) -> u32 {
        // 1 start bit + data bits + parity bit (if any) + stop bits
        1 + match self.data_bits {
            DataBits::Five => 5,
            DataBits::Six => 6,
            DataBits::Seven => 7,
            DataBits::Eight => 8,
        } + match self.parity {
            Parity::Odd | Parity::Even => 1,
            Parity::None => 0,
        } + match self.stop_bits {
            StopBits::One => 1,
            StopBits::Two => 2,
        }
    }

    // Calculates the time to transmit one byte in microseconds.
    fn byte_duration(&self) -> Option<Duration> {
        self.simulate_delay.then(|| {
            Duration::from_micros(((1_000_000 / self.baud_rate) * self.bits_per_byte()) as u64)
        })
    }

    /// Compares relevant physical settings between two configs.
    /// Returns `true` if they don't match, `false` otherwise.
    fn physical_settings_mismatch(&self, other: &Config) -> bool {
        self.baud_rate != other.baud_rate
            || self.data_bits != other.data_bits
            || self.parity != other.parity
            || self.stop_bits != other.stop_bits
    }
}

/// `VirtualPort` simulates a serial port for testing purposes. It supports
/// setting various serial port parameters like baud rate, data bits, flow control,
/// parity, and stop bits. It also supports reading from and writing to buffers.
///
/// Port pair wiring diagram:
///
///  Port 1            Port 2
/// ┌─────┐           ┌─────┐
/// │ TXD ├──────────▶│ RXD │
/// │ RXD │◂──────────┤ TXD │
/// │ RTS ├──────────▶│ CTS │
/// │ CTS │◂──────────┤ RTS │
/// │ DTR ├─────────┬▶│ DSR │
/// │     │         └▶│ CD  │
/// │ DSR │◂┬─────────┤ DTR │
/// │ CD  │◂┘         │     │
/// │ RI  ├───────────┤ RI  │
/// └─────┘           └─────┘
#[derive(Clone)]
pub struct VirtualPort {
    // Configuration settings (baud rate, data bits etc.)
    config: Arc<Mutex<Config>>,

    // Reference to the paired port's configuration
    paired_port_config: Option<Arc<Mutex<Config>>>,

    // Buffers and condition variables for read/write synchronization
    read_buffer: Arc<(Mutex<VecDeque<u8>>, Condvar, Condvar)>,
    write_buffer: Arc<(Mutex<VecDeque<u8>>, Condvar, Condvar)>,

    // Control lines (RTS<-->CTS, DTR<-->DSR/CD)
    // RI (ring indicator) is always true in this implementation
    rts: Arc<Mutex<bool>>,
    cts: Arc<Mutex<bool>>,
    dtr: Arc<Mutex<bool>>,
    dsr_cd: Arc<Mutex<bool>>,
}

impl VirtualPort {
    /// Opens a single loopback virtual port with the specified baud rate.
    pub fn open_loopback(baud_rate: u32, buffer_capacity: usize) -> Result<Self> {
        let buffer = Arc::new((
            Mutex::new(VecDeque::with_capacity(buffer_capacity)),
            Condvar::new(),
            Condvar::new(),
        ));
        let rts_cts = Arc::new(Mutex::new(true));
        let dtr_dsr_cd = Arc::new(Mutex::new(true));

        Ok(Self {
            config: Arc::new(Mutex::new(Config::new(baud_rate))),
            paired_port_config: None,

            read_buffer: buffer.clone(),
            write_buffer: buffer.clone(),

            rts: rts_cts.clone(),
            cts: rts_cts.clone(),
            dtr: dtr_dsr_cd.clone(),
            dsr_cd: dtr_dsr_cd.clone(),
        })
    }

    /// Opens a pair of connected virtual ports with the specified baud rate.
    /// These ports can simulate a communication between two devices.
    pub fn open_pair(baud_rate: u32, buffer_capacity: usize) -> Result<(Self, Self)> {
        let read_buffer = Arc::new((
            Mutex::new(VecDeque::with_capacity(buffer_capacity)),
            Condvar::new(),
            Condvar::new(),
        ));
        let write_buffer = Arc::new((
            Mutex::new(VecDeque::with_capacity(buffer_capacity)),
            Condvar::new(),
            Condvar::new(),
        ));

        let rts = Arc::new(Mutex::new(true));
        let cts = Arc::new(Mutex::new(true));
        let dtr = Arc::new(Mutex::new(true));
        let dsr_cd = Arc::new(Mutex::new(true));

        let mut port1 = Self {
            config: Arc::new(Mutex::new(Config::new(baud_rate))),
            paired_port_config: None,

            read_buffer: read_buffer.clone(),
            write_buffer: write_buffer.clone(),

            rts: rts.clone(),
            cts: cts.clone(),
            dtr: dtr.clone(),
            dsr_cd: dsr_cd.clone(),
        };

        let mut port2 = Self {
            config: Arc::new(Mutex::new(Config::new(baud_rate))),
            paired_port_config: None,

            read_buffer: write_buffer.clone(),
            write_buffer: read_buffer.clone(),

            rts: cts.clone(),
            cts: rts.clone(),
            dtr: dsr_cd.clone(),
            dsr_cd: dtr.clone(),
        };

        port1.paired_port_config = Some(port2.config.clone());
        port2.paired_port_config = Some(port1.config.clone());

        Ok((port1, port2))
    }

    /// Returns whether transmission delay simulation is enabled.
    pub fn simulate_delay(&self) -> bool {
        self.config.lock().unwrap().simulate_delay
    }

    /// Sets whether to simulate the transmission delay for reading operations.
    pub fn set_simulate_delay(&mut self, value: bool) {
        self.config.lock().unwrap().simulate_delay = value;
    }

    /// Returns whether to simulate corrupted symbols if physical settings don't match.
    pub fn noise_on_config_mismatch(&self) -> bool {
        self.config.lock().unwrap().noise_on_config_mismatch
    }

    /// Sets whether to simulate corrupted symbols if physical settings don't match.
    pub fn set_noise_on_config_mismatch(&mut self, value: bool) {
        self.config.lock().unwrap().noise_on_config_mismatch = value;
    }
}

impl io::Read for VirtualPort {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (buffer_mutex, can_read, can_write) = &*self.read_buffer.clone();

        let mut buffer_guard = buffer_mutex.lock().unwrap();

        if buffer_guard.is_empty() {
            let (new_buffer_guard, timeout_result) =
                can_read.wait_timeout(buffer_guard, self.timeout()).unwrap();

            if timeout_result.timed_out() {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }

            buffer_guard = new_buffer_guard;
        }

        let bytes_to_read = buf.len().min(buffer_guard.len());

        for byte in buf.iter_mut().take(bytes_to_read) {
            *byte = buffer_guard.pop_front().unwrap();
        }

        // Lock the configuration once and get necessary parameters
        let (noise_required, delay_per_byte) = {
            let config = self.config.lock().unwrap();

            // Determine if noise simulation is needed
            let noise_required = if config.noise_on_config_mismatch {
                if let Some(paired_port_config) = &self.paired_port_config {
                    let paired_config = paired_port_config.lock().unwrap();
                    config.physical_settings_mismatch(&paired_config)
                } else {
                    false
                }
            } else {
                false
            };

            // Get the delay per byte
            let delay_per_byte = config.byte_duration();

            (noise_required, delay_per_byte)
        };

        // Fill the buffer with noise if required
        if noise_required {
            let mut rng = rand::thread_rng();
            buf.iter_mut()
                .take(bytes_to_read)
                .for_each(|byte| *byte = rng.gen());
        }

        // Notify the writer that space is available
        can_write.notify_one();

        // Simulate the delay of data transmission based on baud rate
        if let Some(delay) = delay_per_byte {
            std::thread::sleep(delay * bytes_to_read as u32);
        }

        Ok(bytes_to_read)
    }
}

impl io::Write for VirtualPort {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let (buffer_mutex, can_read, can_write) = &*self.write_buffer;

        let mut buffer_guard = buffer_mutex.lock().unwrap();

        if buffer_guard.capacity() - buffer_guard.len() == 0 {
            let (new_buffer_guard, timeout_result) = can_write
                .wait_timeout(buffer_guard, self.timeout())
                .unwrap();

            if timeout_result.timed_out() {
                return Err(io::Error::from(io::ErrorKind::TimedOut));
            }

            buffer_guard = new_buffer_guard;
        }

        let bytes_to_write = buf.len().min(buffer_guard.capacity() - buffer_guard.len());

        buffer_guard.extend(&buf[0..bytes_to_write]);

        // Notify the reader that data is available
        can_read.notify_one();

        Ok(bytes_to_write)
    }

    fn flush(&mut self) -> io::Result<()> {
        let (buffer_mutex, _, can_write) = &*self.write_buffer;

        let mut buffer_guard = buffer_mutex.lock().unwrap();

        while buffer_guard.len() > 0 {
            buffer_guard = can_write.wait(buffer_guard).unwrap();
        }

        Ok(())
    }
}

impl SerialPort for VirtualPort {
    fn name(&self) -> Option<String> {
        None
    }

    fn baud_rate(&self) -> Result<u32> {
        Ok(self.config.lock().unwrap().baud_rate)
    }

    fn data_bits(&self) -> Result<DataBits> {
        Ok(self.config.lock().unwrap().data_bits)
    }

    fn flow_control(&self) -> Result<FlowControl> {
        Ok(self.config.lock().unwrap().flow_control)
    }

    fn parity(&self) -> Result<Parity> {
        Ok(self.config.lock().unwrap().parity)
    }

    fn stop_bits(&self) -> Result<StopBits> {
        Ok(self.config.lock().unwrap().stop_bits)
    }

    fn timeout(&self) -> Duration {
        self.config.lock().unwrap().timeout
    }

    fn set_baud_rate(&mut self, baud_rate: u32) -> Result<()> {
        self.config.lock().unwrap().baud_rate = baud_rate;
        Ok(())
    }

    fn set_flow_control(&mut self, flow_control: FlowControl) -> Result<()> {
        self.config.lock().unwrap().flow_control = flow_control;
        Ok(())
    }

    fn set_parity(&mut self, parity: Parity) -> Result<()> {
        self.config.lock().unwrap().parity = parity;
        Ok(())
    }

    fn set_data_bits(&mut self, data_bits: DataBits) -> Result<()> {
        self.config.lock().unwrap().data_bits = data_bits;
        Ok(())
    }

    fn set_stop_bits(&mut self, stop_bits: StopBits) -> Result<()> {
        self.config.lock().unwrap().stop_bits = stop_bits;
        Ok(())
    }

    fn set_timeout(&mut self, timeout: Duration) -> Result<()> {
        self.config.lock().unwrap().timeout = timeout;
        Ok(())
    }

    fn write_request_to_send(&mut self, level: bool) -> Result<()> {
        *self.rts.lock().unwrap() = level;
        Ok(())
    }

    fn write_data_terminal_ready(&mut self, level: bool) -> Result<()> {
        *self.dtr.lock().unwrap() = level;
        Ok(())
    }

    fn read_clear_to_send(&mut self) -> Result<bool> {
        Ok(*self.cts.lock().unwrap())
    }

    fn read_data_set_ready(&mut self) -> Result<bool> {
        Ok(*self.dsr_cd.lock().unwrap())
    }

    fn read_ring_indicator(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn read_carrier_detect(&mut self) -> Result<bool> {
        Ok(*self.dsr_cd.lock().unwrap())
    }

    fn bytes_to_read(&self) -> Result<u32> {
        u32::try_from(self.read_buffer.0.lock().unwrap().len())
            .map_err(|_| Error::new(ErrorKind::Unknown, "buffer is too large"))
    }

    fn bytes_to_write(&self) -> Result<u32> {
        u32::try_from(self.write_buffer.0.lock().unwrap().len())
            .map_err(|_| Error::new(ErrorKind::Unknown, "buffer is too large"))
    }

    fn clear(&self, buffer_to_clear: ClearBuffer) -> Result<()> {
        let (read_buffer, write_buffer) = if Arc::ptr_eq(&self.read_buffer, &self.write_buffer) {
            // If loopback
            (None, Some(&*self.write_buffer))
        } else {
            // If pair
            match buffer_to_clear {
                ClearBuffer::Input => (Some(&*self.read_buffer), None),
                ClearBuffer::Output => (None, Some(&*self.write_buffer)),
                ClearBuffer::All => (Some(&*self.read_buffer), Some(&*self.write_buffer)),
            }
        };

        if let Some(buffer) = read_buffer {
            let (buffer_mutex, _, _) = buffer;
            buffer_mutex.lock().unwrap().clear()
        }

        if let Some(buffer) = write_buffer {
            let (buffer_mutex, _, can_write) = buffer;
            buffer_mutex.lock().unwrap().clear();
            can_write.notify_one();
        }

        Ok(())
    }

    fn try_clone(&self) -> Result<Box<dyn SerialPort>> {
        Ok(Box::new(self.clone()))
    }

    fn set_break(&self) -> Result<()> {
        Ok(())
    }

    fn clear_break(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    #[test]
    fn test_loopback() {
        let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();
        let write_data = b"hello";
        let mut read_data = [0u8; 5];

        assert_eq!(port.write(write_data).unwrap(), 5);
        assert_eq!(port.read(&mut read_data).unwrap(), 5);
        assert_eq!(&read_data, write_data);
    }

    #[test]
    fn test_open_pair() {
        let (mut port1, mut port2) = VirtualPort::open_pair(9600, 1024).unwrap();
        let write_data = b"hello";
        let mut read_data = [0u8; 5];

        assert_eq!(port1.write(write_data).unwrap(), 5);
        assert_eq!(port2.read(&mut read_data).unwrap(), 5);
        assert_eq!(&read_data, write_data);
    }

    #[test]
    fn test_timeout() {
        let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();
        port.set_timeout(Duration::from_millis(100)).unwrap();
        let mut read_data = [0u8; 5];

        assert_eq!(
            port.read(&mut read_data).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn test_control_lines() {
        let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();

        port.write_request_to_send(true).unwrap();
        assert_eq!(port.read_clear_to_send().unwrap(), true);

        port.write_data_terminal_ready(true).unwrap();
        assert_eq!(port.read_data_set_ready().unwrap(), true);

        port.write_request_to_send(false).unwrap();
        assert_eq!(port.read_clear_to_send().unwrap(), false);

        port.write_data_terminal_ready(false).unwrap();
        assert_eq!(port.read_data_set_ready().unwrap(), false);
    }

    #[test]
    fn test_buffer_clearing() {
        let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();
        port.set_timeout(Duration::from_millis(100)).unwrap();
        let write_data = b"test";
        let mut read_data = [0u8; 4];

        port.write(write_data).unwrap();
        port.clear(ClearBuffer::All).unwrap();

        assert_eq!(
            port.read(&mut read_data).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn test_clone() {
        let port = VirtualPort::open_loopback(9600, 1024).unwrap();
        let port_clone = port.try_clone().unwrap();

        assert_eq!(port.baud_rate().unwrap(), port_clone.baud_rate().unwrap());
    }

    #[test]
    fn test_multiple_threads() {
        use std::{thread, time};

        let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();
        let mut port_clone = port.try_clone().unwrap();

        let writer = thread::spawn(move || {
            let write_data = b"hello";
            port.write(write_data).unwrap();
        });

        let reader = thread::spawn(move || {
            let mut read_data = [0u8; 5];
            thread::sleep(time::Duration::from_millis(100));
            port_clone.read(&mut read_data).unwrap();
            assert_eq!(&read_data, b"hello");
        });

        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn test_config_change() {
        let mut port = VirtualPort::open_loopback(9600, 1024).unwrap();

        port.set_baud_rate(19200).unwrap();
        assert_eq!(port.baud_rate().unwrap(), 19200);

        port.set_data_bits(DataBits::Seven).unwrap();
        assert_eq!(port.data_bits().unwrap(), DataBits::Seven);

        port.set_flow_control(FlowControl::Software).unwrap();
        assert_eq!(port.flow_control().unwrap(), FlowControl::Software);

        port.set_parity(Parity::Odd).unwrap();
        assert_eq!(port.parity().unwrap(), Parity::Odd);

        port.set_stop_bits(StopBits::Two).unwrap();
        assert_eq!(port.stop_bits().unwrap(), StopBits::Two);
    }

    #[test]
    fn test_delay_simulation() {
        use std::time::Instant;

        let mut port = VirtualPort::open_loopback(50, 1024).unwrap();

        // Initially, simulate_delay should be false by default
        assert_eq!(port.simulate_delay(), false);

        // Enable simulation delay
        port.set_simulate_delay(true);
        assert_eq!(port.simulate_delay(), true);

        // Write data to the port
        // (for 5 symbols the transmission time is about 1 second for 50 baud rate)
        let write_data = b"hello";
        port.write(write_data).unwrap();

        // Read data from the port and measure duration
        let mut read_data = [0u8; 5];
        let start = Instant::now();
        port.read(&mut read_data).unwrap();
        let duration = start.elapsed();

        assert_eq!(&read_data, write_data);
        assert!(duration.as_millis() > 700);
    }

    #[test]
    fn test_noise_on_config_mismatch() {
        let (mut port1, mut port2) = VirtualPort::open_pair(9600, 1024).unwrap();

        // Initially, noise simulation should be disabled by default
        assert_eq!(port1.noise_on_config_mismatch(), false);
        assert_eq!(port2.noise_on_config_mismatch(), false);

        let write_data = b"hello world";
        let mut read_data = [0u8; 11];

        // Case 1: Verify data transfer when configurations match (noise simulation is not enabled)

        // Write data to port1
        port1.write(write_data).unwrap();

        // Read data from port2
        read_data.fill(0);
        port2.read(&mut read_data).unwrap();

        // Ensure the data in the buffers are equal
        assert_eq!(&read_data, write_data);

        // Case 2: Verify behavior when configurations mismatch (noise simulation is not enabled)

        // Set baud rate to a different value to mismatch configs
        port2.set_baud_rate(19200).unwrap();

        // Write data to port1
        port1.write(write_data).unwrap();

        // Read data from port2
        read_data.fill(0);
        port2.read(&mut read_data).unwrap();

        // Ensure the data in the buffers are equal
        assert_eq!(&read_data, write_data);

        // Case 3: Verify noise simulation when configs match again (noise simulation is enabled)

        // Enable noise simulation for port2
        port2.set_noise_on_config_mismatch(true);
        assert_eq!(port1.noise_on_config_mismatch(), false);
        assert_eq!(port2.noise_on_config_mismatch(), true);

        // Set baud rate to the original value to match configs
        port2.set_baud_rate(port1.baud_rate().unwrap()).unwrap();

        // Write data to port1
        port1.write(write_data).unwrap();

        // Read data from port2
        read_data.fill(0);
        port2.read(&mut read_data).unwrap();

        // Ensure the data in the buffers are equal
        assert_eq!(&read_data, write_data);

        // Case 4: Verify noise simulation when configs mismatch again (noise simulation is enabled)

        // Set baud rate to a different value to mismatch configs
        port2.set_baud_rate(19200).unwrap();

        // Write data to port1
        port1.write(write_data).unwrap();

        // Read data from port2
        read_data.fill(0);
        port2.read(&mut read_data).unwrap();

        // Ensure the buffer differs and contains random data (simple test to check non-zero bytes)
        assert_ne!(&read_data, write_data);
        assert!(read_data.iter().any(|&byte| byte != 0));
    }
}
