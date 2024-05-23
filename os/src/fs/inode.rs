//! `Arc<Inode>` -> `OSInodeInner`: In order to open files concurrently
//! we need to wrap `Inode` into `Arc`,but `Mutex` in `Inode` prevents
//! file systems from being accessed simultaneously
//!
//! `UPSafeCell<OSInodeInner>` -> `OSInode`: for static `ROOT_INODE`,we
//! need to wrap `OSInodeInner` into `UPSafeCell`
use super::File;
use crate::drivers::BLOCK_DEVICE;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::sync::Arc;
use alloc::vec::Vec;
use bitflags::*;
use easy_fs::{EasyFileSystem, Inode};
use lazy_static::*;
use alloc::string::String;
use super::StatMode;
/// inode in memory
/// A wrapper around a filesystem inode
/// to implement File trait atop
pub struct OSInode {
    readable: bool,
    writable: bool,
    file_name: String, 
    inner: UPSafeCell<OSInodeInner>,
}
/// The OS inode inner in 'UPSafeCell'
pub struct OSInodeInner {
    offset: usize,
    inode: Arc<Inode>,
}

impl OSInode {
    /// create a new inode in memory
    pub fn new(readable: bool, writable: bool, file_name:String, inode: Arc<Inode>) -> Self {
        Self {
            readable,
            writable,
            file_name:file_name,
            inner: unsafe { UPSafeCell::new(OSInodeInner { offset: 0, inode }) },
        }
    }
    /// read all data from the inode
    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let mut buffer = [0u8; 512];
        let mut v: Vec<u8> = Vec::new();
        loop {
            let len = inner.inode.read_at(inner.offset, &mut buffer);
            if len == 0 {
                break;
            }
            inner.offset += len;
            v.extend_from_slice(&buffer[..len]);
        }
        v
    }
}

lazy_static! {
    pub static ref ROOT_INODE: Arc<Inode> = {
        let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
        Arc::new(EasyFileSystem::root_inode(&efs))
    };
}

/// List all apps in the root directory
pub fn list_apps() {
    println!("/**** APPS ****");
    for app in ROOT_INODE.ls() {
        println!("{}", app);
    }
    println!("**************/");
}

bitflags! {
    ///  The flags argument to the open() system call is constructed by ORing together zero or more of the following values:
    pub struct OpenFlags: u32 {
        /// readyonly
        const RDONLY = 0;
        /// writeonly
        const WRONLY = 1 << 0;
        /// read and write
        const RDWR = 1 << 1;
        /// create new file
        const CREATE = 1 << 9;
        /// truncate file size to 0
        const TRUNC = 1 << 10;
    }
}

impl OpenFlags {
    /// Do not check validity for simplicity
    /// Return (readable, writable)
    pub fn read_write(&self) -> (bool, bool) {
        if self.is_empty() {
            (true, false)
        } else if self.contains(Self::WRONLY) {
            (false, true)
        } else {
            (true, true)
        }
    }
}

/// Open a file
pub fn open_file(name: &str, flags: OpenFlags) -> Option<Arc<OSInode>> {
    let (readable, writable) = flags.read_write();
    let file_name = String::from(name);
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = ROOT_INODE.find(name) {
            // clear size
            inode.clear();
            Some(Arc::new(OSInode::new(readable, writable, file_name, inode)))
        } else {
            // create file
            ROOT_INODE
                .create(name)
                .map(|inode| Arc::new(OSInode::new(readable, writable, file_name, inode)))
        }
    } else {
        ROOT_INODE.find(name).map(|inode| {
            if flags.contains(OpenFlags::TRUNC) {
                inode.clear();
            }
            Arc::new(OSInode::new(readable, writable, file_name, inode))
        })
    }
}

impl File for OSInode {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, mut buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_read_size = 0usize;
        for slice in buf.buffers.iter_mut() {
            let read_size = inner.inode.read_at(inner.offset, *slice);
            if read_size == 0 {
                break;
            }
            inner.offset += read_size;
            total_read_size += read_size;
        }
        total_read_size
    }
    fn write(&self, buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_write_size = 0usize;
        for slice in buf.buffers.iter() {
            let write_size = inner.inode.write_at(inner.offset, *slice);
            assert_eq!(write_size, slice.len());
            inner.offset += write_size;
            total_write_size += write_size;
        }
        total_write_size
    }

    fn file_name(&self) -> &str {
        &self.file_name
    }

    fn link_number(&self) -> usize{
        let inner = self.inner.exclusive_access();
        inner.inode.read_disk_inode(|disk_inode| {
             disk_inode.nlinks as usize
        })
    }

    fn stat(&self) -> super::Stat {
        let inner = self.inner.exclusive_access();
        inner.inode.read_disk_inode(|disk_inode| {
            super::Stat {
                dev: 0,
                ino:get_inode_id(self.file_name()) as u64,
                mode: if disk_inode.is_dir() {
                    StatMode::DIR
                } else if disk_inode.is_file(){
                    StatMode::FILE
                } else {
                    StatMode::NULL
                },
                nlink: disk_inode.nlinks as u32, 
                pad: [0; 7],
            }
        })
    
    }
}
/// return the inode id
pub fn get_inode_id(file_name: &str) -> i32 {
    if let Some(ino) = ROOT_INODE.read_disk_inode(|disk_inode| {
        ROOT_INODE.find_inode_id(file_name, disk_inode)
    }) {
        ino as i32
    } else {
        -1
    }
}

/// Return the file mode
pub fn get_file_mode(file_name: &str) -> StatMode {
    // Since there just one root directory
    // Just traverse the ROOT_INODE to find the inode
    let inode = ROOT_INODE.find(file_name);
    let mode = match inode {
        Some(inode) => {
            inode.read_disk_inode(|disk_inode|{
                if disk_inode.is_dir() {
                    StatMode::DIR
                } else {
                    StatMode::FILE
                }
            })
        }
        None => StatMode::NULL,
    };
    mode
}

/// call the link function in inode 
pub fn link(oldname: &str, newname: &str) -> isize {
    ROOT_INODE.link(oldname, newname)
}

/// call the unlink function in inode 
pub fn unlink(name: &str) -> isize {
    ROOT_INODE.unlink(name)
}