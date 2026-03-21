/// USB connection to the Rongta RP326.
///
/// The printer enumerates as USB Printer class (bInterfaceClass=7) with:
///   VID = 0x0FE6, PID = 0x811E
///   Bulk OUT endpoint = 0x01

use rusb::{Context, DeviceHandle, UsbContext};
use std::time::Duration;

const VENDOR_ID: u16 = 0x0FE6;
const PRODUCT_ID: u16 = 0x811E;
const BULK_OUT_ENDPOINT: u8 = 0x01;
const INTERFACE: u8 = 0;
const TIMEOUT: Duration = Duration::from_secs(5);

pub struct Printer {
    handle: DeviceHandle<Context>,
}

impl Printer {
    pub fn open() -> anyhow::Result<Self> {
        let ctx = Context::new()?;
        let handle = ctx
            .open_device_with_vid_pid(VENDOR_ID, PRODUCT_ID)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Printer not found (VID=0x{VENDOR_ID:04X} PID=0x{PRODUCT_ID:04X}). \
                     Is it powered on and connected via USB?"
                )
            })?;

        // Detach kernel driver if the OS has claimed the interface (Linux / macOS)
        if handle.kernel_driver_active(INTERFACE)? {
            handle.detach_kernel_driver(INTERFACE)?;
        }

        handle.claim_interface(INTERFACE)?;

        Ok(Self { handle })
    }

    pub fn write(&self, data: &[u8]) -> anyhow::Result<()> {
        let mut offset = 0;
        while offset < data.len() {
            let sent = self
                .handle
                .write_bulk(BULK_OUT_ENDPOINT, &data[offset..], TIMEOUT)?;
            if sent == 0 {
                anyhow::bail!("write_bulk stalled at offset {offset}");
            }
            offset += sent;
        }
        Ok(())
    }
}

impl Drop for Printer {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(INTERFACE);
    }
}
