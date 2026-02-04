use memmap2::{Mmap, MmapMut};
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

pub struct MmapFileMut {
    _file: File,
    mmap: MmapMut,
}

pub struct MmapFile {
    _file: File,
    mmap: Mmap,
}

impl MmapFileMut {
    /// Create a new file to `size_bytes` and map it read-write
    pub fn create_rw<P: AsRef<Path>>(path: P, size_bytes: u64) -> io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        file.set_len(size_bytes)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };
        Ok(Self { _file: file, mmap })
    }

    /// Open an existing file and map it to read and write
    pub fn open_rw<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;

        let mmap = unsafe { MmapMut::map_mut(&file)? };

        Ok(Self { _file: file, mmap })
    }

    /// Return raw pointer to start of memory mapped file data
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.mmap.as_mut_ptr()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.mmap.len()
    }
}
