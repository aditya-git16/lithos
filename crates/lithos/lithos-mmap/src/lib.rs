use memmap2::{Mmap, MmapMut};
use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
};

pub struct MmapFileMut {
    /// File handle kept alive to maintain the memory map validity
    _file: File,
    /// Memory-mapped region providing mutable access to file contents
    mmap: MmapMut,
}

pub struct MmapFile {
    /// File handle kept alive to maintain the memory map validity
    _file: File,
    /// Memory-mapped region providing read-only access to file contents
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

impl MmapFile {
    /// Open an existing file and map it read-only.
    pub fn open_ro<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Self { _file: file, mmap })
    }

    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.mmap.as_ptr()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.mmap.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mmap_roundtrip_bytes() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = format!("/tmp/lithos_mmap_test_{ts}");
        let size = 4096;

        {
            let mut mm = MmapFileMut::create_rw(&path, size).unwrap();
            unsafe {
                let p = mm.as_mut_ptr();
                *p.add(0) = 0xAB;
                *p.add(1) = 0xCD;
            }
        }
        {
            let mm = MmapFile::open_ro(&path).unwrap();
            unsafe {
                let p = mm.as_ptr();
                assert_eq!(*p.add(0), 0xAB);
                assert_eq!(*p.add(1), 0xCD);
            }
        }

        let _ = fs::remove_file(&path);
    }
}
