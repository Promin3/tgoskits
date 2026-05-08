// Copyright 2025 The Axvisor Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Ext4 filesystem implementation backed by ext4plus.

use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    sync::Arc,
    vec,
};
use core::{error::Error, num::NonZeroU32};

use ax_fs_vfs::{
    VfsDirEntry, VfsError, VfsNodeAttr, VfsNodeOps, VfsNodePerm, VfsNodeRef, VfsNodeType, VfsOps,
    VfsResult,
};
use ext4plus::{
    DirEntryName, Ext4, Ext4Read, Ext4Write, FileType, FollowSymlinks,
    dir::Dir as Ext4Dir,
    error::Ext4Error,
    file::File as Ext4File,
    inode::{Inode as Ext4Inode, InodeCreationOptions, InodeFlags, InodeMode},
    path::Path as Ext4Path,
};
use spin::Mutex;

use crate::dev::{Disk, Partition};

const EXT4_ROOT_INO: u32 = 2;
const VFS_DIRENT_NAME_LEN: usize = 63;

/// Ext4 filesystem implementation.
pub struct Ext4FileSystem {
    state: Arc<Mutex<Ext4State>>,
}

struct Ext4State {
    fs: Ext4,
    disk: Arc<Ext4Disk>,
}

struct Ext4Disk {
    dev: Mutex<Ext4Device>,
}

enum Ext4Device {
    Disk(Disk),
    Partition(Partition),
}

struct Ext4Node {
    state: Arc<Mutex<Ext4State>>,
    path: String,
    ino: u32,
}

impl Ext4FileSystem {
    /// Create a new ext4 filesystem from a disk device.
    #[allow(dead_code)]
    pub fn new(disk: Disk) -> VfsResult<Self> {
        info!(
            "Got Disk size:{}, position:{}",
            disk.size(),
            disk.position()
        );
        Self::load(Arc::new(Ext4Disk::new_disk(disk)))
    }

    /// Create a new ext4 filesystem from a partition.
    pub fn from_partition(partition: Partition) -> VfsResult<Self> {
        info!(
            "Got Partition size:{}, position:{}",
            partition.size(),
            partition.position()
        );
        Self::load(Arc::new(Ext4Disk::new_partition(partition)))
    }

    fn load(disk: Arc<Ext4Disk>) -> VfsResult<Self> {
        disk.ext4_block_size()?;
        let fs = Ext4::load_with_writer(Box::new(disk.clone()), Some(Box::new(disk.clone())))
            .map_err(into_vfs_err)?;
        Ok(Self {
            state: Arc::new(Mutex::new(Ext4State { fs, disk })),
        })
    }
}

impl VfsOps for Ext4FileSystem {
    fn root_dir(&self) -> VfsNodeRef {
        Arc::new(Ext4Node::new(
            self.state.clone(),
            String::from("/"),
            EXT4_ROOT_INO,
        ))
    }
}

impl Ext4Disk {
    fn new_disk(disk: Disk) -> Self {
        Self {
            dev: Mutex::new(Ext4Device::Disk(disk)),
        }
    }

    fn new_partition(partition: Partition) -> Self {
        Self {
            dev: Mutex::new(Ext4Device::Partition(partition)),
        }
    }

    fn ext4_block_size(&self) -> VfsResult<u32> {
        let mut data = [0; 4];
        self.transfer(1024 + 0x18, &mut data, None)
            .map_err(|_| VfsError::Io)?;
        let log_block_size = u32::from_le_bytes(data);
        1024u32
            .checked_shl(log_block_size)
            .filter(|size| matches!(*size, 1024 | 2048 | 4096))
            .ok_or(VfsError::InvalidData)
    }

    fn flush(&self) -> VfsResult {
        Ok(())
    }

    fn transfer(
        &self,
        start_byte: u64,
        dst: &mut [u8],
        write_src: Option<&[u8]>,
    ) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
        if let Some(src) = write_src
            && src.len() != dst.len()
        {
            return Err(Box::new(Ext4DiskError));
        }

        let mut dev = self.dev.lock();
        let disk_size = dev.size();
        let end = start_byte
            .checked_add(dst.len() as u64)
            .ok_or_else(|| Box::new(Ext4DiskError))?;
        if end > disk_size {
            return Err(Box::new(Ext4DiskError));
        }

        dev.set_position(start_byte);
        let mut done = 0usize;
        while done < dst.len() {
            let count = if let Some(src) = write_src {
                dev.write_one(&src[done..])?
            } else {
                dev.read_one(&mut dst[done..])?
            };
            if count == 0 {
                return Err(Box::new(Ext4DiskError));
            }
            done += count;
        }
        Ok(())
    }
}

impl Ext4Device {
    fn size(&self) -> u64 {
        match self {
            Self::Disk(disk) => disk.size(),
            Self::Partition(partition) => partition.size(),
        }
    }

    fn set_position(&mut self, pos: u64) {
        match self {
            Self::Disk(disk) => disk.set_position(pos),
            Self::Partition(partition) => partition.set_position(pos),
        }
    }

    fn read_one(&mut self, buf: &mut [u8]) -> Result<usize, Ext4DiskError> {
        match self {
            Self::Disk(disk) => disk.read_one(buf),
            Self::Partition(partition) => partition.read_one(buf),
        }
        .map_err(|_| Ext4DiskError)
    }

    fn write_one(&mut self, buf: &[u8]) -> Result<usize, Ext4DiskError> {
        match self {
            Self::Disk(disk) => disk.write_one(buf),
            Self::Partition(partition) => partition.write_one(buf),
        }
        .map_err(|_| Ext4DiskError)
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

impl Ext4Node {
    fn new(state: Arc<Mutex<Ext4State>>, path: String, ino: u32) -> Self {
        Self { state, path, ino }
    }

    fn inode_index(&self) -> VfsResult<NonZeroU32> {
        NonZeroU32::new(self.ino).ok_or(VfsError::InvalidData)
    }

    fn read_inode(&self, state: &Ext4State) -> VfsResult<Ext4Inode> {
        Ext4Inode::read(&state.fs, self.inode_index()?).map_err(into_vfs_err)
    }

    fn absolute_child_path(&self, path: &str) -> String {
        let raw = if path.starts_with('/') {
            path.to_string()
        } else if self.path == "/" {
            format!("/{path}")
        } else {
            format!("{}/{}", self.path.trim_end_matches('/'), path)
        };
        let canonical = ax_fs_vfs::path::canonicalize(&raw);
        if canonical.is_empty() {
            String::from("/")
        } else if canonical.starts_with('/') {
            canonical
        } else {
            format!("/{canonical}")
        }
    }

    fn split_parent(path: &str) -> VfsResult<(&str, &str)> {
        let path = path.trim_end_matches('/');
        if path.is_empty() || path == "/" {
            return Err(VfsError::InvalidInput);
        }
        let (parent, name) = path.rsplit_once('/').ok_or(VfsError::InvalidInput)?;
        if name.is_empty() || name == "." || name == ".." {
            return Err(VfsError::InvalidInput);
        }
        Ok((if parent.is_empty() { "/" } else { parent }, name))
    }

    fn parent_path(&self) -> Option<String> {
        if self.path == "/" {
            return None;
        }
        let (parent, _) = Self::split_parent(&self.path).ok()?;
        Some(parent.to_string())
    }

    fn create_entry(state: Arc<Mutex<Ext4State>>, path: String, inode: Ext4Inode) -> VfsNodeRef {
        Arc::new(Self::new(state, path, inode.index.get()))
    }

    fn current_time() -> core::time::Duration {
        if cfg!(feature = "times") {
            ax_hal::time::wall_time()
        } else {
            core::time::Duration::default()
        }
    }
}

impl VfsNodeOps for Ext4Node {
    fn get_attr(&self) -> VfsResult<VfsNodeAttr> {
        let state = self.state.lock();
        let inode = self.read_inode(&state)?;
        let metadata = inode.metadata();
        Ok(VfsNodeAttr::new(
            VfsNodePerm::from_bits_truncate(metadata.mode()),
            file_type_to_vfs(metadata.file_type()),
            metadata.len(),
            inode.blocks(),
        ))
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> VfsResult<usize> {
        let state = self.state.lock();
        let inode = self.read_inode(&state)?;
        match inode.file_type() {
            FileType::Directory => Err(VfsError::IsADirectory),
            FileType::Symlink => {
                let target = inode.symlink_target(&state.fs).map_err(into_vfs_err)?;
                read_bytes_at(target.as_ref(), buf, offset)
            }
            FileType::Regular => {
                let mut file = Ext4File::open_inode(&state.fs, inode).map_err(into_vfs_err)?;
                file.read_bytes_at(buf, offset).map_err(into_vfs_err)
            }
            _ => Err(VfsError::InvalidInput),
        }
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> VfsResult<usize> {
        let state = self.state.lock();
        let inode = self.read_inode(&state)?;
        if inode.file_type() != FileType::Regular {
            return Err(VfsError::InvalidInput);
        }
        let mut file = Ext4File::open_inode(&state.fs, inode).map_err(into_vfs_err)?;
        let written = file.write_bytes_at(buf, offset).map_err(into_vfs_err)?;
        file.into_inode().write(&state.fs).map_err(into_vfs_err)?;
        state.disk.flush()?;
        Ok(written)
    }

    fn fsync(&self) -> VfsResult {
        self.state.lock().disk.flush()
    }

    fn truncate(&self, size: u64) -> VfsResult {
        let state = self.state.lock();
        let inode = self.read_inode(&state)?;
        if inode.file_type() != FileType::Regular {
            return Err(VfsError::InvalidInput);
        }
        let mut file = Ext4File::open_inode(&state.fs, inode).map_err(into_vfs_err)?;
        file.truncate(size).map_err(into_vfs_err)?;
        file.into_inode().write(&state.fs).map_err(into_vfs_err)?;
        state.disk.flush()
    }

    fn parent(&self) -> Option<VfsNodeRef> {
        let parent_path = self.parent_path()?;
        let state = self.state.lock();
        let inode = state
            .fs
            .path_to_inode(ext4_path(&parent_path).ok()?, FollowSymlinks::All)
            .ok()?;
        Some(Self::create_entry(self.state.clone(), parent_path, inode))
    }

    fn lookup(self: Arc<Self>, path: &str) -> VfsResult<VfsNodeRef> {
        debug!("lookup at ext4plus: {}", path);
        let path = self.absolute_child_path(path);
        let state = self.state.lock();
        let inode = state
            .fs
            .path_to_inode(ext4_path(&path)?, FollowSymlinks::All)
            .map_err(into_vfs_err)?;
        Ok(Self::create_entry(self.state.clone(), path, inode))
    }

    fn create(&self, path: &str, ty: VfsNodeType) -> VfsResult {
        debug!("create {:?} at ext4plus: {}", ty, path);
        let path = self.absolute_child_path(path);
        let (parent_path, name) = Self::split_parent(&path)?;
        let state = self.state.lock();
        if state.fs.exists(ext4_path(&path)?).map_err(into_vfs_err)? {
            return Ok(());
        }

        let parent_inode = state
            .fs
            .path_to_inode(ext4_path(parent_path)?, FollowSymlinks::All)
            .map_err(into_vfs_err)?;
        let parent_index = parent_inode.index;
        let mut parent = Ext4Dir::open_inode(&state.fs, parent_inode).map_err(into_vfs_err)?;
        if ty == VfsNodeType::Dir {
            let inode = state
                .fs
                .create_inode(InodeCreationOptions {
                    file_type: FileType::Directory,
                    mode: mode_for(ty)?,
                    uid: 0,
                    gid: 0,
                    time: Self::current_time(),
                    flags: InodeFlags::empty(),
                })
                .map_err(into_vfs_err)?;
            let mut dir =
                Ext4Dir::init(state.fs.clone(), inode, parent_index).map_err(into_vfs_err)?;
            parent
                .link(dir_name(name)?, dir.inode_mut())
                .map_err(into_vfs_err)?;
        } else if ty == VfsNodeType::SymLink {
            return Err(VfsError::Unsupported);
        } else {
            let mut inode = state
                .fs
                .create_inode(InodeCreationOptions {
                    file_type: vfs_type_to_file_type(ty)?,
                    mode: mode_for(ty)?,
                    uid: 0,
                    gid: 0,
                    time: Self::current_time(),
                    flags: InodeFlags::empty(),
                })
                .map_err(into_vfs_err)?;
            parent
                .link(dir_name(name)?, &mut inode)
                .map_err(into_vfs_err)?;
        }
        state.disk.flush()
    }

    fn remove(&self, path: &str) -> VfsResult {
        debug!("remove at ext4plus: {}", path);
        let path = self.absolute_child_path(path);
        let (parent_path, name) = Self::split_parent(&path)?;
        let state = self.state.lock();
        let parent_inode = state
            .fs
            .path_to_inode(ext4_path(parent_path)?, FollowSymlinks::All)
            .map_err(into_vfs_err)?;
        let mut parent = Ext4Dir::open_inode(&state.fs, parent_inode).map_err(into_vfs_err)?;
        let mut target = parent.get_entry(dir_name(name)?).map_err(into_vfs_err)?;
        if target.file_type() == FileType::Directory {
            let old = parent.inode().links_count();
            parent.inode_mut().set_links_count(old.saturating_sub(1));
            parent.inode_mut().write(&state.fs).map_err(into_vfs_err)?;
            target.set_links_count(1);
        }
        parent
            .unlink(dir_name(name)?, target)
            .map_err(into_vfs_err)?;
        state.disk.flush()
    }

    fn read_dir(&self, start_idx: usize, dirents: &mut [VfsDirEntry]) -> VfsResult<usize> {
        let state = self.state.lock();
        let reader = state
            .fs
            .read_dir(ext4_path(&self.path)?)
            .map_err(into_vfs_err)?;
        let mut idx = 0usize;
        let mut count = 0usize;
        for entry in reader {
            let entry = entry.map_err(into_vfs_err)?;
            let file_name = entry.file_name();
            let name =
                core::str::from_utf8(file_name.as_ref()).map_err(|_| VfsError::InvalidData)?;
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            if idx < start_idx {
                idx += 1;
                continue;
            }
            if count >= dirents.len() {
                break;
            }
            if name.len() > VFS_DIRENT_NAME_LEN {
                return Err(VfsError::NameTooLong);
            }
            let ty = file_type_to_vfs(entry.file_type().map_err(into_vfs_err)?);
            dirents[count] = VfsDirEntry::new(name, ty);
            idx += 1;
            count += 1;
        }
        Ok(count)
    }

    fn rename(&self, src_path: &str, dst_path: &str) -> VfsResult {
        debug!(
            "rename at ext4plus, src_path: {}, dst_path: {}",
            src_path, dst_path
        );
        let src_path = self.absolute_child_path(src_path);
        let dst_path = self.absolute_child_path(dst_path);
        if src_path == dst_path {
            return Ok(());
        }
        let (src_parent_path, src_name) = Self::split_parent(&src_path)?;
        let (dst_parent_path, dst_name) = Self::split_parent(&dst_path)?;
        let state = self.state.lock();
        let src_parent_inode = state
            .fs
            .path_to_inode(ext4_path(src_parent_path)?, FollowSymlinks::All)
            .map_err(into_vfs_err)?;
        let dst_parent_inode = state
            .fs
            .path_to_inode(ext4_path(dst_parent_path)?, FollowSymlinks::All)
            .map_err(into_vfs_err)?;
        let mut src_parent =
            Ext4Dir::open_inode(&state.fs, src_parent_inode).map_err(into_vfs_err)?;
        let mut dst_parent =
            Ext4Dir::open_inode(&state.fs, dst_parent_inode).map_err(into_vfs_err)?;
        let mut src = src_parent
            .get_entry(dir_name(src_name)?)
            .map_err(into_vfs_err)?;
        if src.file_type() == FileType::Directory {
            return Err(VfsError::Unsupported);
        }
        if let Ok(dst) = dst_parent.get_entry(dir_name(dst_name)?) {
            if src.index == dst.index {
                return Ok(());
            }
            dst_parent
                .unlink(dir_name(dst_name)?, dst)
                .map_err(into_vfs_err)?;
        }
        dst_parent
            .link(dir_name(dst_name)?, &mut src)
            .map_err(into_vfs_err)?;
        src_parent
            .unlink(dir_name(src_name)?, src)
            .map_err(into_vfs_err)?;
        state.disk.flush()
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

fn ext4_path(path: &str) -> VfsResult<Ext4Path<'_>> {
    Ext4Path::try_from(path).map_err(|_| VfsError::InvalidInput)
}

fn dir_name(name: &str) -> VfsResult<DirEntryName<'_>> {
    DirEntryName::try_from(name).map_err(|_| VfsError::InvalidInput)
}

fn read_bytes_at(src: &[u8], dst: &mut [u8], offset: u64) -> VfsResult<usize> {
    let offset = usize::try_from(offset).map_err(|_| VfsError::InvalidInput)?;
    if offset >= src.len() {
        return Ok(0);
    }
    let len = dst.len().min(src.len() - offset);
    dst[..len].copy_from_slice(&src[offset..offset + len]);
    Ok(len)
}

fn file_type_to_vfs(file_type: FileType) -> VfsNodeType {
    match file_type {
        FileType::BlockDevice => VfsNodeType::BlockDevice,
        FileType::CharacterDevice => VfsNodeType::CharDevice,
        FileType::Directory => VfsNodeType::Dir,
        FileType::Fifo => VfsNodeType::Fifo,
        FileType::Regular => VfsNodeType::File,
        FileType::Socket => VfsNodeType::Socket,
        FileType::Symlink => VfsNodeType::SymLink,
    }
}

fn vfs_type_to_file_type(node_type: VfsNodeType) -> VfsResult<FileType> {
    Ok(match node_type {
        VfsNodeType::BlockDevice => FileType::BlockDevice,
        VfsNodeType::CharDevice => FileType::CharacterDevice,
        VfsNodeType::Dir => FileType::Directory,
        VfsNodeType::Fifo => FileType::Fifo,
        VfsNodeType::File => FileType::Regular,
        VfsNodeType::Socket => FileType::Socket,
        VfsNodeType::SymLink => FileType::Symlink,
    })
}

fn mode_for(node_type: VfsNodeType) -> VfsResult<InodeMode> {
    let ty = match node_type {
        VfsNodeType::BlockDevice => InodeMode::S_IFBLK,
        VfsNodeType::CharDevice => InodeMode::S_IFCHR,
        VfsNodeType::Dir => InodeMode::S_IFDIR,
        VfsNodeType::Fifo => InodeMode::S_IFIFO,
        VfsNodeType::File => InodeMode::S_IFREG,
        VfsNodeType::Socket => InodeMode::S_IFSOCK,
        VfsNodeType::SymLink => InodeMode::S_IFLNK,
    };
    let perm = if node_type == VfsNodeType::Dir {
        VfsNodePerm::default_dir()
    } else {
        VfsNodePerm::default_file()
    };
    Ok(ty | InodeMode::from_bits_retain(perm.bits()))
}

fn into_vfs_err(err: Ext4Error) -> VfsError {
    match err {
        Ext4Error::NotAbsolute | Ext4Error::MalformedPath | Ext4Error::InvalidXattrName => {
            VfsError::InvalidInput
        }
        Ext4Error::NotASymlink | Ext4Error::IsASpecialFile => VfsError::InvalidData,
        Ext4Error::NotFound => VfsError::NotFound,
        Ext4Error::IsADirectory => VfsError::IsADirectory,
        Ext4Error::NotADirectory => VfsError::NotADirectory,
        Ext4Error::FileTooLarge => VfsError::StorageFull,
        Ext4Error::NotUtf8 => VfsError::InvalidData,
        Ext4Error::PathTooLong => VfsError::NameTooLong,
        Ext4Error::TooManySymlinks => VfsError::FilesystemLoop,
        Ext4Error::Encrypted | Ext4Error::Readonly => VfsError::PermissionDenied,
        Ext4Error::Io(_) | Ext4Error::Corrupt(_) | Ext4Error::Incompatible(_) => VfsError::Io,
        Ext4Error::UnsupportedOperation(_) => VfsError::Unsupported,
        Ext4Error::NoSpace => VfsError::StorageFull,
        Ext4Error::AlreadyExists => VfsError::AlreadyExists,
        Ext4Error::DotEntry => VfsError::InvalidInput,
        _ => VfsError::Io,
    }
}
