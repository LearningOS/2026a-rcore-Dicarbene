use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ, NAME_LENGTH_LIMIT,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};
/// Virtual filesystem layer over easy-fs
pub struct Inode {
    block_id: usize,
    block_offset: usize,
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
    file_id: Option<u32>,
}

impl Inode {
    /// Create a vfs inode
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
            file_id: None,
        }
    }
    /// Call a function over a disk inode to read it
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// Call a function over a disk inode to modify it
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// Find inode under a disk inode by name
    fn entries<'a>(&'a self, disk_inode: &'a DiskInode) -> impl Iterator<Item = DirEntry> + 'a {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        (0..file_count).map(move |i| {
            let mut dirent = DirEntry::empty();
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            dirent
        })
    }
    fn find_entry(&self, name: &str, disk_inode: &DiskInode) -> Option<(usize, u32)> {
        self.entries(disk_inode)
            .enumerate()
            .find(|(_, entry)| !entry.name().is_empty() && entry.name() == name)
            .map(|(index, entry)| (index, entry.inode_id()))
    }
    fn open_inode(&self, fs: &mut EasyFileSystem, id: u32) -> Arc<Inode> {
        fs.open_files.entry(id).or_default().handles += 1;
        let (block_id, block_offset) = fs.get_disk_inode_pos(id);
        let mut inode = Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        );
        inode.file_id = Some(id);
        Arc::new(inode)
    }
    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let (_, id) = self.read_disk_inode(|inode| self.find_entry(name, inode))?;
        Some(self.open_inode(&mut fs, id))
    }
    /// Increase the size of a disk inode
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }
    /// Create inode under current inode by name
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        if !valid_name(name) {
            return None;
        }
        let mut fs = self.fs.lock();
        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_entry(name, root_inode)
        };
        if self.read_disk_inode(op).is_some() {
            return None;
        }
        // create a new file
        // alloc a inode with an indirect block
        let new_inode_id = fs.alloc_inode();
        // initialize inode
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        self.insert_entry(name, new_inode_id, &mut fs);
        block_cache_sync_all();
        Some(self.open_inode(&mut fs, new_inode_id))
    }
    fn insert_entry(&self, name: &str, id: u32, fs: &mut MutexGuard<EasyFileSystem>) {
        self.modify_disk_inode(|directory| {
            let index = self
                .entries(directory)
                .position(|entry| entry.name().is_empty())
                .unwrap_or(directory.size as usize / DIRENT_SZ);
            self.increase_size(((index + 1) * DIRENT_SZ) as u32, directory, fs);
            directory.write_at(
                index * DIRENT_SZ,
                DirEntry::new(name, id).as_bytes(),
                &self.block_device,
            );
        });
    }
    /// Add another name for an existing file. Lab4 implementation assisted by Codex.
    /// https://learningos.cn/rCore-Tutorial-Guide/chapter6/4exercise.html
    pub fn link(&self, old: &str, new: &str) -> Option<()> {
        if !valid_name(new) {
            return None;
        }
        let mut fs = self.fs.lock();
        let id = self.read_disk_inode(|directory| {
            if self.find_entry(new, directory).is_some() {
                return None;
            }
            self.find_entry(old, directory).map(|(_, id)| id)
        })?;
        self.insert_entry(new, id, &mut fs);
        block_cache_sync_all();
        Some(())
    }
    /// Remove a name and reclaim the file after its last name and handle are gone.
    pub fn unlink(&self, name: &str) -> Option<()> {
        let mut fs = self.fs.lock();
        let (id, last_link) = self.modify_disk_inode(|directory| {
            let (index, id) = self.find_entry(name, directory)?;
            directory.write_at(
                index * DIRENT_SZ,
                DirEntry::empty().as_bytes(),
                &self.block_device,
            );
            let last = self.count_links(directory, id) == 0;
            Some((id, last))
        })?;
        if last_link {
            if let Some(open) = fs.open_files.get_mut(&id) {
                open.unlinked = true;
            } else {
                fs.reclaim_file(id);
            }
        }
        block_cache_sync_all();
        Some(())
    }
    fn count_links(&self, directory: &DiskInode, id: u32) -> u32 {
        self.entries(directory)
            .filter(|entry| entry.inode_id() == id && !entry.name().is_empty())
            .count() as u32
    }
    /// Return the on-disk inode number and its number of names in this directory.
    pub fn file_stat(&self, file: &Inode) -> Option<(u32, u32)> {
        let id = file.file_id?;
        let _fs = self.fs.lock();
        Some((
            id,
            self.read_disk_inode(|directory| self.count_links(directory, id)),
        ))
    }
    /// List inodes under current inode
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.entries(disk_inode)
                .filter(|entry| !entry.name().is_empty())
                .map(|entry| String::from(entry.name()))
                .collect()
        })
    }
    /// Read data from current inode
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write data to current inode
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }
    /// Clear the data in current inode
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }
}

impl Drop for Inode {
    fn drop(&mut self) {
        if let Some(id) = self.file_id {
            let mut fs = self.fs.lock();
            let open = fs.open_files.get_mut(&id).unwrap();
            open.handles -= 1;
            if open.handles == 0 {
                let unlinked = fs.open_files.remove(&id).unwrap().unlinked;
                if unlinked {
                    fs.reclaim_file(id);
                    block_cache_sync_all();
                }
            }
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= NAME_LENGTH_LIMIT && !name.contains(['\0', '/'])
}
