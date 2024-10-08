use std::{ any::TypeId, marker::PhantomData };

pub const PAGE_SIZE_BYTES: usize = 64;
pub struct PageAllocator {
    memory: Vec<u8>, // Contiguous memory
    page_size: usize, // Size of each page
    total_pages: usize, // Total number of pages
    free_list: Vec<usize>, // List of free pages (holds offsets)
}
pub struct FixedDataPtr<T> {
    page_ptr: usize,
    data_size: usize,
    type_id: TypeId,
    _phantom: PhantomData<T>,
}

impl<T: 'static> FixedDataPtr<T> {
    pub fn new(page_ptr: usize) -> Self {
        Self {
            page_ptr,
            data_size: size_of::<T>(),
            type_id: TypeId::of::<T>(),
            _phantom: PhantomData,
        }
    }

    pub fn cast<U: 'static>(self) -> Option<FixedDataPtr<U>> {
        if TypeId::of::<U>() == self.type_id {
            Some(FixedDataPtr {
                page_ptr: self.page_ptr,
                data_size: self.data_size,
                type_id: self.type_id,
                _phantom: PhantomData,
            })
        } else {
            None
        }
    }
}

pub struct DynamicDataPtr {
    page_ptr: usize,
    capacity: usize, // to find out how many pages are occupied
    len: usize,
}
impl PageAllocator {
    pub fn new(total_size: usize, page_size: usize) -> Self {
        let total_pages = total_size / page_size;
        let mut memory = Vec::with_capacity(total_size);
        memory.resize(total_size, 0); // Fill memory with zeros
        let free_list = (0..total_pages).map(|p| p * page_size).collect(); // Initialize free list with page offsets
        PageAllocator {
            memory,
            page_size,
            total_pages,
            free_list,
        }
    }

    pub fn alloc_fixed(&mut self, size: usize) -> Option<FixedDataPtr> {
        let start_offset = self.free_list.pop()?;
        Some(FixedDataPtr::new(start_offset))
    }

    pub fn dealloc_fixed<T>(&mut self, ptr: FixedDataPtr<T>) {
        self.free_list.push(ptr.page_ptr);
    }

    pub fn write_fixed_to_memory<T: Copy + 'static, U: 'static>(
        &mut self,
        ptr: &FixedDataPtr<T>,
        data: &U
    ) -> FixedDataPtr<U> {
        let start = ptr.page_ptr;
        let new_size = size_of::<U>();
        let end = start + new_size;

        if end > self.memory.len() {
            panic!("Memory access out of bounds");
        }

        unsafe {
            let src = data as *const U as *const u8;
            let dst = self.memory[start..end].as_mut_ptr();
            std::ptr::copy_nonoverlapping(src, dst, new_size);
        }

        FixedDataPtr {
            page_ptr: start,
            data_size: new_size,
            type_id: TypeId::of::<U>(),
            _phantom: PhantomData,
        }
    }

    pub fn read_fixed_from_memory<T: Copy + 'static>(&self, ptr: &FixedDataPtr<T>) -> T {
        let start = ptr.page_ptr;
        let end = start + ptr.data_size;

        if end > self.memory.len() {
            panic!("Memory access out of bounds");
        }

        if TypeId::of::<T>() != ptr.type_id {
            panic!("Type mismatch: trying to read a different type than what was stored");
        }

        unsafe {
            let src = self.memory[start..end].as_ptr() as *const T;
            std::ptr::read(src)
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Clone, Copy)]
    struct Player {
        id: u32,
        health: u32,
    }

    #[test]
    fn test_page_allocator_allocation() {
        let mut allocator = PageAllocator::new(1024 * 1024, 64);
        let offset = allocator.alloc(64).expect("Failed to allocate a page");

        assert!(offset.0 < allocator.total_pages, "Invalid page offset");

        allocator.dealloc(offset, 64); // Pass size to dealloc
        assert_eq!(
            allocator.free_list.len(),
            allocator.total_pages,
            "Page was not deallocated properly"
        );
    }

    #[test]
    fn test_write_and_read_struct() {
        let mut allocator = PageAllocator::new(1024 * 1024, 4096); // 1 MB total, 4KB page size
        let player = Player { id: 42, health: 200 };
        let offset = allocator
            .alloc(std::mem::size_of::<Player>())
            .expect("Failed to allocate memory for player");

        allocator.write_to_memory(offset, &player);
        let read_player: Player = allocator.read_from_memory(offset);

        assert_eq!(player, read_player, "Written and read Player struct do not match");
        allocator.dealloc(offset, std::mem::size_of::<Player>()); // Pass size to dealloc
    }

    #[test]
    fn test_multiple_allocations() {
        let mut allocator = PageAllocator::new(1024 * 1024, 4096); // 1 MB total, 4KB page size
        let offset1 = allocator
            .alloc(std::mem::size_of::<Player>())
            .expect("Failed to allocate first page");
        let offset2 = allocator
            .alloc(std::mem::size_of::<Player>())
            .expect("Failed to allocate second page");

        assert!(offset1 != offset2, "Allocated the same page twice");

        let player1 = Player { id: 1, health: 100 };
        let player2 = Player { id: 2, health: 150 };

        allocator.write_to_memory(offset1, &player1);
        allocator.write_to_memory(offset2, &player2);

        let read_player1: Player = allocator.read_from_memory(offset1);
        let read_player2: Player = allocator.read_from_memory(offset2);

        assert_eq!(player1, read_player1, "Player 1 mismatch");
        assert_eq!(player2, read_player2, "Player 2 mismatch");

        allocator.dealloc(offset1, std::mem::size_of::<Player>()); // Pass size to dealloc
        allocator.dealloc(offset2, std::mem::size_of::<Player>()); // Pass size to dealloc
    }

    #[test]
    fn test_exhaust_all_pages() {
        let mut allocator = PageAllocator::new(4096 * 2, 4096); // 2 pages total, 4KB page size
        let offset1 = allocator.alloc(4096).expect("Failed to allocate first page");
        let offset2 = allocator.alloc(4096).expect("Failed to allocate second page");
        let offset3 = allocator.alloc(4096);

        assert!(offset3.is_none(), "Allocated more pages than available");

        allocator.dealloc(offset1, 4096); // Pass size to dealloc
        allocator.dealloc(offset2, 4096); // Pass size to dealloc

        assert_eq!(allocator.free_list.len(), 2, "Not all pages were deallocated");
    }
}
