use alloc::{
    string::{String, ToString},
    sync::Arc,
};
use core::{any::Any, num::NonZeroU32};

use axfs_ng_vfs::{
    DeviceId, DirEntry, DirEntrySink, DirNode, DirNodeOps, FileNode, FileNodeOps, FilesystemOps,
    Metadata, MetadataUpdate, NodeFlags, NodeOps, NodePermission, NodeType, Reference, VfsError,
    VfsResult, WeakDirEntry,
};
use axpoll::{IoEvents, Pollable};
use ext4plus::{
    DirEntryName, FileType, FollowSymlinks, Metadata as Ext4Metadata,
    dir::Dir as Ext4Dir,
    file::File as Ext4File,
    inode::{Inode as Ext4Inode, InodeCreationOptions, InodeFlags, InodeMode},
    path::{Path as Ext4Path, PathBuf as Ext4PathBuf},
};

use super::{
    Ext4Filesystem,
    util::{file_type_to_vfs, into_vfs_err, mode_for, vfs_type_to_file_type},
};

pub struct Inode {
    fs: Arc<Ext4Filesystem>,
    ino: u32,
    this: Option<WeakDirEntry>,
    path: spin::Mutex<Option<String>>,
}

impl Inode {
    pub(crate) fn new(
        fs: Arc<Ext4Filesystem>,
        ino: u32,
        this: Option<WeakDirEntry>,
        path: Option<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            fs,
            ino,
            this,
            path: spin::Mutex::new(path),
        })
    }

    fn inode_index(&self) -> VfsResult<NonZeroU32> {
        NonZeroU32::new(self.ino).ok_or(VfsError::InvalidData)
    }

    fn dir_path(&self) -> VfsResult<String> {
        if let Some(this) = self.this.as_ref().and_then(WeakDirEntry::upgrade) {
            return Ok(this.absolute_path()?.to_string());
        }
        self.path.lock().clone().ok_or(VfsError::InvalidInput)
    }

    fn create_entry(&self, name: impl Into<String>, inode: Ext4Inode) -> DirEntry {
        let name = name.into();
        let ino = inode.index.get();
        let node_type = file_type_to_vfs(inode.file_type());
        let reference = Reference::new(
            self.this.as_ref().and_then(WeakDirEntry::upgrade),
            name.clone(),
        );
        let path = self.dir_path().map(|dir| join_child_path(&dir, &name)).ok();
        if node_type == NodeType::Directory {
            DirEntry::new_dir(
                |this| DirNode::new(Self::new(self.fs.clone(), ino, Some(this), path.clone())),
                reference,
            )
        } else {
            DirEntry::new_file(
                FileNode::new(Self::new(self.fs.clone(), ino, None, path)),
                node_type,
                reference,
            )
        }
    }

    fn read_inode(&self, state: &super::fs::Ext4State) -> VfsResult<Ext4Inode> {
        Ext4Inode::read(&state.fs, self.inode_index()?).map_err(into_vfs_err)
    }

    fn update_ctime(inode: &mut Ext4Inode) {
        if cfg!(feature = "times") {
            inode.set_ctime(ax_hal::time::wall_time());
        }
    }

    fn current_time() -> core::time::Duration {
        if cfg!(feature = "times") {
            ax_hal::time::wall_time()
        } else {
            core::time::Duration::default()
        }
    }
}

impl NodeOps for Inode {
    fn inode(&self) -> u64 {
        self.ino as u64
    }

    fn metadata(&self) -> VfsResult<Metadata> {
        let state = self.fs.lock();
        let inode = self.read_inode(&state)?;
        let metadata = inode.metadata();
        Ok(to_vfs_metadata(self.ino, state.block_size as u64, metadata))
    }

    fn update_metadata(&self, update: MetadataUpdate) -> VfsResult<()> {
        let state = self.fs.lock();
        let mut inode = self.read_inode(&state)?;
        if let Some(mode) = update.mode {
            inode
                .set_mode(
                    (inode.mode() & InodeMode::from_bits_retain(!0o777))
                        | InodeMode::from_bits_retain(mode.bits()),
                )
                .map_err(into_vfs_err)?;
        }
        if let Some((uid, gid)) = update.owner {
            inode.set_uid(uid);
            inode.set_gid(gid);
        }
        if let Some(atime) = update.atime {
            inode.set_atime(atime);
        }
        if let Some(mtime) = update.mtime {
            inode.set_mtime(mtime);
        }
        Self::update_ctime(&mut inode);
        inode.write(&state.fs).map_err(into_vfs_err)?;
        state.disk.flush().map_err(|_| VfsError::Io)
    }

    fn len(&self) -> VfsResult<u64> {
        let state = self.fs.lock();
        Ok(self.read_inode(&state)?.size_in_bytes())
    }

    fn filesystem(&self) -> &dyn FilesystemOps {
        &*self.fs
    }

    fn sync(&self, _data_only: bool) -> VfsResult<()> {
        self.fs.flush_disk()
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn flags(&self) -> NodeFlags {
        NodeFlags::BLOCKING
    }
}

impl FileNodeOps for Inode {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> VfsResult<usize> {
        let state = self.fs.lock();
        let inode = self.read_inode(&state)?;
        if inode.file_type() == FileType::Symlink {
            let target = inode.symlink_target(&state.fs).map_err(into_vfs_err)?;
            return read_bytes_at(target.as_ref(), buf, offset);
        }
        let mut file = Ext4File::open_inode(&state.fs, inode).map_err(into_vfs_err)?;
        file.read_bytes_at(buf, offset).map_err(into_vfs_err)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> VfsResult<usize> {
        let state = self.fs.lock();
        let inode = self.read_inode(&state)?;
        let mut file = Ext4File::open_inode(&state.fs, inode).map_err(into_vfs_err)?;
        let written = file.write_bytes_at(buf, offset).map_err(into_vfs_err)?;
        let mut inode = file.into_inode();
        Self::update_ctime(&mut inode);
        inode.write(&state.fs).map_err(into_vfs_err)?;
        state.disk.flush().map_err(|_| VfsError::Io)?;
        Ok(written)
    }

    fn append(&self, buf: &[u8]) -> VfsResult<(usize, u64)> {
        let state = self.fs.lock();
        let inode = self.read_inode(&state)?;
        let old_len = inode.size_in_bytes();
        let mut file = Ext4File::open_inode(&state.fs, inode).map_err(into_vfs_err)?;
        let written = file.write_bytes_at(buf, old_len).map_err(into_vfs_err)?;
        let mut inode = file.into_inode();
        Self::update_ctime(&mut inode);
        inode.write(&state.fs).map_err(into_vfs_err)?;
        state.disk.flush().map_err(|_| VfsError::Io)?;
        Ok((written, old_len + written as u64))
    }

    fn set_len(&self, len: u64) -> VfsResult<()> {
        let state = self.fs.lock();
        let inode = self.read_inode(&state)?;
        let mut file = Ext4File::open_inode(&state.fs, inode).map_err(into_vfs_err)?;
        file.truncate(len).map_err(into_vfs_err)?;
        let mut inode = file.into_inode();
        Self::update_ctime(&mut inode);
        inode.write(&state.fs).map_err(into_vfs_err)?;
        state.disk.flush().map_err(|_| VfsError::Io)
    }

    fn set_symlink(&self, _target: &str) -> VfsResult<()> {
        let state = self.fs.lock();
        let inode = self.read_inode(&state)?;
        if inode.file_type() != FileType::Symlink {
            return Err(VfsError::InvalidInput);
        }
        Err(VfsError::Unsupported)
    }
}

impl Pollable for Inode {
    fn poll(&self) -> IoEvents {
        IoEvents::IN | IoEvents::OUT
    }

    fn register(&self, _context: &mut core::task::Context<'_>, _events: IoEvents) {}
}

impl DirNodeOps for Inode {
    fn read_dir(&self, offset: u64, sink: &mut dyn DirEntrySink) -> VfsResult<usize> {
        let path = self.dir_path()?;
        let state = self.fs.lock();
        let reader = state.fs.read_dir(ext4_path(&path)?).map_err(into_vfs_err)?;
        let mut idx = 0u64;
        let mut count = 0usize;
        for entry in reader {
            let entry = entry.map_err(into_vfs_err)?;
            if idx < offset {
                idx += 1;
                continue;
            }
            let file_name = entry.file_name();
            let name =
                core::str::from_utf8(file_name.as_ref()).map_err(|_| VfsError::InvalidData)?;
            let node_type = file_type_to_vfs(entry.file_type().map_err(into_vfs_err)?);
            idx += 1;
            if !sink.accept(name, entry.inode.get() as u64, node_type, idx) {
                return Ok(count);
            }
            count += 1;
        }
        Ok(count)
    }

    fn lookup(&self, name: &str) -> VfsResult<DirEntry> {
        if name == "." {
            return self
                .this
                .as_ref()
                .and_then(WeakDirEntry::upgrade)
                .ok_or(VfsError::NotFound);
        }
        if name == ".." {
            return self
                .this
                .as_ref()
                .and_then(WeakDirEntry::upgrade)
                .and_then(|entry| entry.parent())
                .ok_or(VfsError::NotFound);
        }
        let path = join_child_path(&self.dir_path()?, name);
        let state = self.fs.lock();
        let inode = state
            .fs
            .path_to_inode(ext4_path(&path)?, FollowSymlinks::ExcludeFinalComponent)
            .map_err(into_vfs_err)?;
        Ok(self.create_entry(name, inode))
    }

    fn create(
        &self,
        name: &str,
        node_type: NodeType,
        permission: NodePermission,
    ) -> VfsResult<DirEntry> {
        let dir_path = self.dir_path()?;
        let path = join_child_path(&dir_path, name);
        let state = self.fs.lock();
        if state.fs.exists(ext4_path(&path)?).map_err(into_vfs_err)? {
            return Err(VfsError::AlreadyExists);
        }
        let parent_inode = self.read_inode(&state)?;
        let mut parent = Ext4Dir::open_inode(&state.fs, parent_inode).map_err(into_vfs_err)?;
        let inode = if node_type == NodeType::Directory {
            let inode = state
                .fs
                .create_inode(InodeCreationOptions {
                    file_type: FileType::Directory,
                    mode: mode_for(node_type, permission)?,
                    uid: 0,
                    gid: 0,
                    time: Self::current_time(),
                    flags: InodeFlags::empty(),
                })
                .map_err(into_vfs_err)?;
            let mut dir = Ext4Dir::init(state.fs.clone(), inode, self.inode_index()?)
                .map_err(into_vfs_err)?;
            parent
                .link(dir_name(name)?, dir.inode_mut())
                .map_err(into_vfs_err)?;
            dir.inode().clone()
        } else if node_type == NodeType::Symlink {
            return Err(VfsError::Unsupported);
        } else {
            let file_type = vfs_type_to_file_type(node_type)?;
            let mut inode = state
                .fs
                .create_inode(InodeCreationOptions {
                    file_type,
                    mode: mode_for(node_type, permission)?,
                    uid: 0,
                    gid: 0,
                    time: Self::current_time(),
                    flags: InodeFlags::empty(),
                })
                .map_err(into_vfs_err)?;
            parent
                .link(dir_name(name)?, &mut inode)
                .map_err(into_vfs_err)?;
            inode
        };
        state.disk.flush().map_err(|_| VfsError::Io)?;
        Ok(self.create_entry(name, inode))
    }

    fn symlink(&self, name: &str, target: &str, permission: NodePermission) -> VfsResult<DirEntry> {
        let dir_path = self.dir_path()?;
        let path = join_child_path(&dir_path, name);
        let state = self.fs.lock();
        if state.fs.exists(ext4_path(&path)?).map_err(into_vfs_err)? {
            return Err(VfsError::AlreadyExists);
        }
        let parent_inode = self.read_inode(&state)?;
        let mut parent = Ext4Dir::open_inode(&state.fs, parent_inode).map_err(into_vfs_err)?;
        let mut inode = state
            .fs
            .symlink(
                &mut parent,
                dir_name(name)?,
                Ext4PathBuf::try_from(target).map_err(|_| VfsError::InvalidInput)?,
                0,
                0,
                Self::current_time(),
            )
            .map_err(into_vfs_err)?;
        inode
            .set_mode(mode_for(NodeType::Symlink, permission)?)
            .map_err(into_vfs_err)?;
        inode.write(&state.fs).map_err(into_vfs_err)?;
        state.disk.flush().map_err(|_| VfsError::Io)?;
        Ok(self.create_entry(name, inode))
    }

    fn link(&self, name: &str, node: &DirEntry) -> VfsResult<DirEntry> {
        let dir_path = self.dir_path()?;
        let link_path = join_child_path(&dir_path, name);
        let state = self.fs.lock();
        if state
            .fs
            .exists(ext4_path(&link_path)?)
            .map_err(into_vfs_err)?
        {
            return Err(VfsError::AlreadyExists);
        }
        let parent_inode = self.read_inode(&state)?;
        let mut parent = Ext4Dir::open_inode(&state.fs, parent_inode).map_err(into_vfs_err)?;
        let mut target = Ext4Inode::read(
            &state.fs,
            NonZeroU32::new(node.inode() as u32).ok_or(VfsError::InvalidData)?,
        )
        .map_err(into_vfs_err)?;
        if target.file_type() == FileType::Directory {
            return Err(VfsError::PermissionDenied);
        }
        parent
            .link(dir_name(name)?, &mut target)
            .map_err(into_vfs_err)?;
        state.disk.flush().map_err(|_| VfsError::Io)?;
        Ok(self.create_entry(name, target))
    }

    fn unlink(&self, name: &str) -> VfsResult<()> {
        let state = self.fs.lock();
        let parent_inode = self.read_inode(&state)?;
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
        state.disk.flush().map_err(|_| VfsError::Io)
    }

    fn rename(&self, src_name: &str, dst_dir: &DirNode, dst_name: &str) -> VfsResult<()> {
        let dst_dir: Arc<Self> = dst_dir.downcast().map_err(|_| VfsError::InvalidInput)?;
        if self.ino == dst_dir.ino && src_name == dst_name {
            return Ok(());
        }
        let state = self.fs.lock();
        let src_parent_inode = self.read_inode(&state)?;
        let dst_parent_inode = dst_dir.read_inode(&state)?;
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
        state.disk.flush().map_err(|_| VfsError::Io)
    }
}

fn to_vfs_metadata(ino: u32, block_size: u64, metadata: Ext4Metadata) -> Metadata {
    Metadata {
        inode: ino as u64,
        device: 0,
        nlink: metadata.links_count as u64,
        mode: NodePermission::from_bits_truncate(metadata.mode()),
        node_type: file_type_to_vfs(metadata.file_type()),
        uid: metadata.uid(),
        gid: metadata.gid(),
        size: metadata.len(),
        block_size,
        blocks: metadata.size_in_bytes.div_ceil(512),
        rdev: DeviceId::default(),
        atime: metadata.atime,
        mtime: metadata.mtime,
        ctime: metadata.ctime,
    }
}

fn ext4_path(path: &str) -> VfsResult<Ext4Path<'_>> {
    Ext4Path::try_from(path).map_err(|_| VfsError::InvalidInput)
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

fn dir_name(name: &str) -> VfsResult<DirEntryName<'_>> {
    DirEntryName::try_from(name).map_err(|_| VfsError::InvalidInput)
}

fn join_child_path(parent: &str, name: &str) -> String {
    if parent == "/" {
        alloc::format!("/{name}")
    } else {
        alloc::format!("{parent}/{name}")
    }
}
