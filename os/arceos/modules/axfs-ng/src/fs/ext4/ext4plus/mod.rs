mod fs;
mod inode;
mod util;

use alloc::{boxed::Box, vec};
use core::error::Error;

use ax_driver::{AxBlockDevice, PartitionBlockDevice, PartitionRegion, prelude::BlockDriverOps};
use axfs_ng_vfs::VfsError;
use ext4plus::{Ext4Read, Ext4Write};
pub use fs::*;
pub use inode::*;

pub(crate) struct Ext4Disk {
    dev: spin::Mutex<PartitionBlockDevice<AxBlockDevice>>,
}

impl Ext4Disk {
    pub(crate) fn new(dev: AxBlockDevice, region: PartitionRegion) -> Self {
        Self {
            dev: spin::Mutex::new(PartitionBlockDevice::new(dev, region)),
        }
    }

    pub(crate) fn flush(&self) -> Result<(), VfsError> {
        self.dev.lock().flush().map_err(|_| VfsError::Io)
    }

    pub(crate) fn ext4_block_size(&self) -> Result<u32, VfsError> {
        let mut data = [0; 4];
        self.transfer(1024 + 0x18, &mut data, None)
            .map_err(|_| VfsError::Io)?;
        let log_block_size = u32::from_le_bytes(data);
        1024u32
            .checked_shl(log_block_size)
            .filter(|size| matches!(*size, 1024 | 2048 | 4096))
            .ok_or(VfsError::InvalidData)
    }

    fn transfer(
        &self,
        start_byte: u64,
        buf: &mut [u8],
        write_src: Option<&[u8]>,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let mut dev = self.dev.lock();
        let block_size = dev.block_size();
        if block_size == 0 {
            return Err(Box::new(Ext4DiskError));
        }
        let disk_size = dev
            .num_blocks()
            .checked_mul(block_size as u64)
            .ok_or_else(|| Box::new(Ext4DiskError))?;
        let end = start_byte
            .checked_add(buf.len() as u64)
            .ok_or_else(|| Box::new(Ext4DiskError))?;
        if end > disk_size {
            return Err(Box::new(Ext4DiskError));
        }

        let mut done = 0usize;
        let mut pos = start_byte;
        let mut block = vec![0u8; block_size];
        while done < buf.len() {
            let block_id = pos / block_size as u64;
            let block_offset = (pos % block_size as u64) as usize;
            let len = (buf.len() - done).min(block_size - block_offset);
            if block_offset == 0 && len == block_size {
                if let Some(src) = write_src {
                    dev.write_block(block_id, &src[done..done + len])
                        .map_err(|_| Box::new(Ext4DiskError))?;
                } else {
                    dev.read_block(block_id, &mut buf[done..done + len])
                        .map_err(|_| Box::new(Ext4DiskError))?;
                }
            } else {
                dev.read_block(block_id, &mut block)
                    .map_err(|_| Box::new(Ext4DiskError))?;
                if let Some(src) = write_src {
                    block[block_offset..block_offset + len].copy_from_slice(&src[done..done + len]);
                    dev.write_block(block_id, &block)
                        .map_err(|_| Box::new(Ext4DiskError))?;
                } else {
                    buf[done..done + len].copy_from_slice(&block[block_offset..block_offset + len]);
                }
            }
            done += len;
            pos += len as u64;
        }
        Ok(())
    }
}

impl Ext4Read for Ext4Disk {
    fn read(
        &self,
        start_byte: u64,
        dst: &mut [u8],
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        self.transfer(start_byte, dst, None)
    }
}

impl Ext4Write for Ext4Disk {
    fn write(
        &self,
        start_byte: u64,
        src: &[u8],
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        let mut scratch = vec![0u8; src.len()];
        self.transfer(start_byte, &mut scratch, Some(src))
    }
}

#[derive(Debug)]
struct Ext4DiskError;

impl core::fmt::Display for Ext4DiskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ext4 disk I/O error")
    }
}

impl Error for Ext4DiskError {}
