use std::{ any::TypeId, marker::PhantomData };

pub const PAGE_SIZE_BYTES: usize = 64;
pub struct PageAllocator {
    memory: Vec<u8>, // Contiguous memory
    page_size: usize, // Size of each page
    total_pages: usize, // Total number of pages
    free_list: Vec<usize>, // List of free pages (holds offsets)
}
#[derive(Debug, Clone, Copy)]
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
        debug_assert!(total_size > page_size);
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
    pub fn get_copy_of_state(&self) -> Vec<u8> {
        return self.memory.clone();
    }
    pub fn alloc_fixed<T: 'static>(&mut self) -> Option<FixedDataPtr<T>> {
        let start = self.free_list.pop();
        if let Some(start) = start {
            return Some(FixedDataPtr::new(start));
        }
        return None;
    }

    pub fn dealloc_fixed<T>(&mut self, ptr: FixedDataPtr<T>) {
        self.free_list.push(ptr.page_ptr);
    }
    pub fn alloc_and_write_fixed<T: Copy + 'static>(
        &mut self,
        data: &T
    ) -> Option<FixedDataPtr<T>> {
        let ptr = self.alloc_fixed::<T>();
        if let Some(ptr) = ptr {
            return Some(self.write_fixed_to_memory(&ptr, data));
        }
        return None;
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
            panic!("PageAllocator access out of bounds");
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
            panic!("PageAllocator access out of bounds");
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
mod tests {
    use super::*;

    #[test]
    fn test_allocation_deallocation() {
        let mut allocator = PageAllocator::new(1024, PAGE_SIZE_BYTES);

        let ptr1 = allocator.alloc_fixed::<i32>();
        assert!(ptr1.is_some());

        let ptr2 = allocator.alloc_fixed::<i32>();
        assert!(ptr2.is_some());

        // Deallocate one page and check if it's reused
        if let Some(ptr) = ptr1 {
            allocator.dealloc_fixed(ptr);
        }

        let recycled_ptr = allocator.alloc_fixed::<i32>();
        assert!(recycled_ptr.is_some());
        assert_eq!(ptr1.unwrap().page_ptr, recycled_ptr.unwrap().page_ptr);
    }

    #[test]
    fn test_casting_between_types() {
        let mut allocator = PageAllocator::new(1024, PAGE_SIZE_BYTES);
        let ptr = allocator.alloc_fixed::<i32>().unwrap();

        // Cast to the same type should succeed
        let casted_ptr = ptr.cast::<i32>();
        assert!(casted_ptr.is_some());

        // Cast to a different type should fail
        let casted_ptr_fail = ptr.cast::<f64>();
        assert!(casted_ptr_fail.is_none());
    }

    #[test]
    fn test_memory_write_and_read() {
        let mut allocator = PageAllocator::new(1024, PAGE_SIZE_BYTES);
        let data = 42u32;

        let ptr = allocator.alloc_and_write_fixed(&data);
        assert!(ptr.is_some());

        if let Some(ptr) = ptr {
            let read_value = allocator.read_fixed_from_memory(&ptr);
            assert_eq!(read_value, data);
        }
    }

    #[test]
    #[should_panic(expected = "PageAllocator access out of bounds")]
    fn test_out_of_bounds_access() {
        let mut allocator = PageAllocator::new(1024, PAGE_SIZE_BYTES);
        let data = [0u8; 128]; // Larger than a page

        // This should panic because it exceeds the page size
        allocator.alloc_and_write_fixed(&data);
    }
}
