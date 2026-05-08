use axfs_ng_vfs::{NodePermission, NodeType, VfsError};
use ext4plus::{FileType, error::Ext4Error, inode::InodeMode};

pub(crate) fn into_vfs_err(err: Ext4Error) -> VfsError {
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

pub(crate) fn file_type_to_vfs(file_type: FileType) -> NodeType {
    match file_type {
        FileType::BlockDevice => NodeType::BlockDevice,
        FileType::CharacterDevice => NodeType::CharacterDevice,
        FileType::Directory => NodeType::Directory,
        FileType::Fifo => NodeType::Fifo,
        FileType::Regular => NodeType::RegularFile,
        FileType::Socket => NodeType::Socket,
        FileType::Symlink => NodeType::Symlink,
    }
}

pub(crate) fn vfs_type_to_file_type(node_type: NodeType) -> VfsErrorResult<FileType> {
    Ok(match node_type {
        NodeType::BlockDevice => FileType::BlockDevice,
        NodeType::CharacterDevice => FileType::CharacterDevice,
        NodeType::Directory => FileType::Directory,
        NodeType::Fifo => FileType::Fifo,
        NodeType::RegularFile => FileType::Regular,
        NodeType::Socket => FileType::Socket,
        NodeType::Symlink => FileType::Symlink,
        NodeType::Unknown => return Err(VfsError::InvalidInput),
    })
}

pub(crate) fn mode_for(
    node_type: NodeType,
    permission: NodePermission,
) -> VfsErrorResult<InodeMode> {
    let ty = match node_type {
        NodeType::BlockDevice => InodeMode::S_IFBLK,
        NodeType::CharacterDevice => InodeMode::S_IFCHR,
        NodeType::Directory => InodeMode::S_IFDIR,
        NodeType::Fifo => InodeMode::S_IFIFO,
        NodeType::RegularFile => InodeMode::S_IFREG,
        NodeType::Socket => InodeMode::S_IFSOCK,
        NodeType::Symlink => InodeMode::S_IFLNK,
        NodeType::Unknown => return Err(VfsError::InvalidInput),
    };
    Ok(ty | InodeMode::from_bits_retain(permission.bits()))
}

type VfsErrorResult<T> = Result<T, VfsError>;
