#![no_std]
#![no_main]
#![feature(const_trait_impl)]
#![feature(never_type)]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt::Write;
use core::mem;
use core::str;

use sel4cp::memory_region::{memory_region_symbol, ExternallySharedRef, ReadOnly, ReadWrite};
use sel4cp::message::{MessageInfo, NoMessageLabel, StatusMessageLabel};
use sel4cp::{protection_domain, Channel, Handler};

use banscii_artist_interface_types as artist;
use banscii_assistant_core::Draft;
use uart_interface_types as driver;

use embedded_hal::serial::{Read as SerialRead, Write as SerialWrite};

const UART_DRIVER: Channel = Channel::new(0);
const TALENT: Channel = Channel::new(1);

const REGION_SIZE: usize = 0x4_000;

const MAX_SUBJECT_LEN: usize = 16;

#[protection_domain(heap_size = 0x10000)]
fn init() -> impl Handler {
    let region_in = unsafe {
        ExternallySharedRef::<'static, [u8]>::new_read_only(
            memory_region_symbol!(region_in_start: *mut [u8], n = REGION_SIZE),
        )
    };

    let region_out = unsafe {
        ExternallySharedRef::<'static, [u8]>::new(
            memory_region_symbol!(region_out_start: *mut [u8], n = REGION_SIZE),
        )
    };
    let mut serial = driver::SerialDriver::new(UART_DRIVER);

    prompt(&mut serial);

    ThisHandler {
        region_in,
        region_out,
        serial,
        buffer: Vec::new(),
    }
}

struct ThisHandler {
    region_in: ExternallySharedRef<'static, [u8], ReadOnly>,
    region_out: ExternallySharedRef<'static, [u8], ReadWrite>,
    serial: driver::SerialDriver,
    buffer: Vec<u8>,
}

impl Handler for ThisHandler {
    type Error = !;

    fn notified(&mut self, channel: Channel) -> Result<(), Self::Error> {
        if channel == self.serial.channel {
            while let Ok(b) = self.serial.read() {
                if let b'\n' | b'\r' = b {
                    newline(&mut self.serial);
                    if !self.buffer.is_empty() {
                        self.try_create();
                    }
                    prompt(&mut self.serial);
                } else {
                    let c = char::from(b);
                    if c.is_ascii() && !c.is_ascii_control() {
                        if self.buffer.len() == MAX_SUBJECT_LEN {
                            writeln!(self.serial, "\n(char limit reached)").unwrap();
                            self.try_create();
                            prompt(&mut self.serial);
                        }
                        let _ = self.serial.write(b);
                        self.buffer.push(b);
                    }
                }
            }
        } else {
            unreachable!()
        }
        Ok(())
    }
}

impl ThisHandler {
    fn try_create(&mut self) {
        let mut buffer = Vec::new();
        mem::swap(&mut buffer, &mut self.buffer);
        match str::from_utf8(&buffer) {
            Ok(subject) => {
                self.create(&subject);
            }
            Err(_) => {
                writeln!(self.serial, "error: input is not valid utf-8").unwrap();
            }
        };
        self.buffer.clear();
    }

    fn create(&mut self, subject: &str) {
        let draft = Draft::new(subject);

        let draft_start = 0;
        let draft_size = draft.pixel_data.len();
        let draft_end = draft_start + draft_size;

        self.region_out
            .as_mut_ptr()
            .index(draft_start..draft_end)
            .copy_from_slice(&draft.pixel_data);

        let msg_info = TALENT.pp_call(MessageInfo::send(
            NoMessageLabel,
            artist::Request {
                height: draft.height,
                width: draft.width,
                draft_start,
                draft_size,
            },
        ));

        assert_eq!(msg_info.label().try_into(), Ok(StatusMessageLabel::Ok));

        let msg = msg_info.recv::<artist::Response>().unwrap();

        let height = msg.height;
        let width = msg.width;

        let pixel_data = self
            .region_in
            .as_ptr()
            .index(msg.masterpiece_start..msg.masterpiece_start + msg.masterpiece_size)
            .copy_to_vec();

        let signature = self
            .region_in
            .as_ptr()
            .index(msg.signature_start..msg.signature_start + msg.signature_size)
            .copy_to_vec();

        newline(&mut self.serial);

        for row in 0..height {
            for col in 0..width {
                let i = row * width + col;
                let b = pixel_data[i];
                let _ = self.serial.write(b);
            }
            newline(&mut self.serial);
        }

        newline(&mut self.serial);

        writeln!(self.serial, "Signature:").unwrap();
        for line in signature.chunks(32) {
            writeln!(self.serial, "{}", hex::encode(line)).unwrap();
        }

        newline(&mut self.serial);
    }
}

fn prompt(serial: &mut driver::SerialDriver) {
    write!(serial, "banscii> ").unwrap();
}

fn newline(serial: &mut driver::SerialDriver) {
    writeln!(serial, "").unwrap();
}
