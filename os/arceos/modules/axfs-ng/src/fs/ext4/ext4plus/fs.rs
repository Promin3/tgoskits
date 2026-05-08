use alloc::{boxed::Box, sync::Arc};

use ax_driver::{AxBlockDevice, PartitionRegion};
use ax_kspin::{SpinNoPreempt as Mutex, SpinNoPreemptGuard as MutexGuard};
use axfs_ng_vfs::{
    DirEntry, DirNode, Filesystem, FilesystemOps, Reference, StatFs, VfsResult, path::MAX_NAME_LEN,
};
use ext4plus::Ext4;

use super::{Ext4Disk, Inode, util::into_vfs_err};

const EXT4_ROOT_INO: u32 = 2;

pub(crate) struct Ext4State {
    pub fs: Ext4,
    pub disk: Arc<Ext4Disk>,
    pub block_size: u32,
}

pub struct Ext4Filesystem {
    inner: Mutex<Ext4State>,
    root_dir: Mutex<Option<DirEntry>>,
}

impl Ext4Filesystem {
    pub fn new(dev: AxBlockDevice, region: PartitionRegion) -> VfsResult<Filesystem> {
        let disk = Arc::new(Ext4Disk::new(dev, region));
        let block_size = disk.ext4_block_size()?;
        let fs = Ext4::load_with_writer(Box::new(disk.clone()), Some(Box::new(disk.clone())))
            .map_err(into_vfs_err)?;
        let result = Arc::new(Self {
            inner: Mutex::new(Ext4State {
                fs,
                disk,
                block_size,
            }),
            root_dir: Mutex::default(),
        });
        let root = DirEntry::new_dir(
            |this| DirNode::new(Inode::new(result.clone(), EXT4_ROOT_INO, Some(this), None)),
            Reference::root(),
        );
        *result.root_dir.lock() = Some(root);
        Ok(Filesystem::new(result))
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Ext4State> {
        self.inner.lock()
    }

    pub(crate) fn flush_disk(&self) -> VfsResult<()> {
        self.inner.lock().disk.flush()
    }
}

impl FilesystemOps for Ext4Filesystem {
    fn name(&self) -> &str {
        "ext4"
    }

    fn root_dir(&self) -> DirEntry {
        self.root_dir.lock().clone().unwrap()
    }

    fn stat(&self) -> VfsResult<StatFs> {
        let state = self.lock();
        let superblock = state.fs.superblock();
        let block_size = state.block_size;
        let blocks = superblock.blocks_count();
        let blocks_free = superblock.free_blocks_count();
        let free_file_count = superblock.free_inodes_count() as u64;
        let file_count = superblock
            .inodes_per_block_group()
            .get()
            .saturating_mul(superblock.num_block_groups()) as u64;
        Ok(StatFs {
            fs_type: 0xef53,
            block_size,
            blocks,
            blocks_free,
            blocks_available: blocks_free,
            file_count,
            free_file_count,
            name_length: MAX_NAME_LEN as _,
            fragment_size: block_size,
            mount_flags: 0,
        })
    }

    fn flush(&self) -> VfsResult<()> {
        self.flush_disk()
    }
}
