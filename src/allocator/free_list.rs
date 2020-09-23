use super::align_up;
use core::alloc::Layout;
use core::mem::{align_of, size_of};
use core::ptr::NonNull;

#[derive(Debug, Clone, Copy)]
struct HoleInfo {
    addr: usize,
    size: usize,
}

#[derive(Debug, Clone, Copy)]
struct Allocation {
    info: HoleInfo,
    front_padding: Option<HoleInfo>,
    back_padding: Option<HoleInfo>,
}

pub(super) struct FreeList {
    head: FreeNode,
    allocated_space: usize,
    free_space: usize,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy)]
pub(super) struct AlignedLayout(Layout);

impl From<AlignedLayout> for Layout {
    fn from(al: AlignedLayout) -> Self {
        al.0
    }
}

impl AlignedLayout {
    pub fn layout(&self) -> &Layout {
        &self.0
    }

    pub fn align(&self) -> usize {
        self.layout().align()
    }

    pub fn size(&self) -> usize {
        self.layout().size()
    }
}

impl FreeList {
    pub const fn min_alignment() -> usize {
        align_of::<FreeNode>()
    }

    pub const fn min_alloc_size() -> usize {
        size_of::<FreeNode>()
    }

    pub fn align_layout(layout: Layout) -> Option<AlignedLayout> {
        // Fixing up the layout in here is useful because we do it before allocation and deallocation,
        // which can simplify things. It makes life a lot easier if the minimal alignment is the same
        // as our free node, and that the size makes sure the end of the allocation is aligned. We also
        // avoid allocating anything smaller than our free node,
        let required_alignment = layout.align();
        let required_alignment = required_alignment.max(Self::min_alignment());
        assert!(required_alignment >= Self::min_alignment());

        let required_size = layout.size();
        let required_size = align_up(
            required_size.max(Self::min_alloc_size()),
            Self::min_alignment(),
        );

        Layout::from_size_align(required_size, required_alignment).ok().map(|l| AlignedLayout(l))
    }

    pub unsafe fn new(start: usize, limit: usize) -> Self {
        // This should be a no-op since the allocation should come from the page
        // allocator and be page aligned, but it does not hurt to be safe
        let aligned_start = align_up(start, align_of::<FreeNode>());
        // And we should definitely be able to fit a free node in the list
        let size = limit.saturating_sub(aligned_start);
        assert!(
            size >= size_of::<FreeNode>(),
            "Buffer size {} is too small",
            size
        );

        let ptr = aligned_start as *mut FreeNode;
        ptr.write(FreeNode { size, next: None });

        FreeList {
            head: FreeNode {
                size: 0,
                next: Some(&mut *ptr),
            },
            allocated_space: 0,
            free_space: size,
        }
    }

    pub fn allocate(&mut self, layout: AlignedLayout) -> Option<NonNull<u8>> {
        Self::tail_allocate(&mut self.head, layout).map(|allocation| {
            if let Some(front_padding) = allocation.front_padding {
                Self::deallocate_from_hole_info(&mut self.head, front_padding);
            }

            if let Some(back_padding) = allocation.back_padding {
                Self::deallocate_from_hole_info(&mut self.head, back_padding);
            }

            self.allocated_space += allocation.info.size;
            self.free_space -= allocation.info.size;
            NonNull::new(allocation.info.addr as *mut u8).unwrap()
        })
    }

    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: AlignedLayout) {
        Self::deallocate_from_hole_info(
            &mut self.head,
            HoleInfo {
                addr: ptr.as_ptr() as usize,
                size: layout.size(),
            },
        );
        self.allocated_space -= layout.size();
        self.free_space += layout.size();
    }

    pub fn free_space(&self) -> usize {
        self.free_space
    }

    pub fn allocated_space(&self) -> usize {
        self.allocated_space
    }

    #[cfg(test)]
    pub fn node_count(&self) -> usize {
        let mut prev_node = &self.head;
        let mut count = 0;
        loop {
            if let Some(next_node) = prev_node.next.as_ref() {
                prev_node = next_node;
                count += 1;
            } else {
                return count;
            }
        }
    }

    fn tail_allocate(mut prev_node: &mut FreeNode, layout: AlignedLayout) -> Option<Allocation> {
        loop {
            let allocation = prev_node
                .next
                .as_mut()
                .and_then(|current| Self::allocate_from_hole_info(current.info(), layout));
            if let Some(allocation) = allocation {
                // So, remove the free node from the list and return the allocation descriptor
                let remove_node = prev_node.next.as_mut().unwrap();
                prev_node.next = remove_node.next.take();
                return Some(allocation);
            } else if prev_node.next.is_some() {
                prev_node = prev_node.next.as_mut().unwrap();
            } else {
                // Out of memory
                return None;
            }
        }
    }

    fn allocate_from_hole_info(hole: HoleInfo, layout: AlignedLayout) -> Option<Allocation> {
        let available_size = hole.size;
        let required_size = layout.size();
        let required_alignment = layout.align();

        // We make some assumptions about alignment of free nodes
        assert!(required_size >= size_of::<FreeNode>());
        assert!(required_alignment >= align_of::<FreeNode>());
        assert!(required_size == align_up(required_size, align_of::<FreeNode>()));

        let node_start = hole.addr;
        let node_end = node_start + available_size;
        let (aligned_hole_start, front_padding) = if align_up(node_start, required_alignment)
            == node_start
        {
            // No need for front padding
            (node_start, None)
        } else {
            // So we can't use the start of the allocation because it isn't suitably aligned. Allow
            // space for a free node
            let aligned_start = align_up(node_start + size_of::<FreeNode>(), required_alignment);
            (
                aligned_start,
                Some(HoleInfo {
                    addr: node_start,
                    size: aligned_start - node_start,
                }),
            )
        };

        let aligned_hole = {
            if aligned_hole_start + required_size > node_end {
                return None;
            }
            HoleInfo {
                addr: aligned_hole_start,
                size: node_end - aligned_hole_start,
            }
        };

        let back_padding = if aligned_hole.size == required_size {
            // No need for any back padding
            None
        } else if aligned_hole.size - required_size < size_of::<FreeNode>() {
            return None;
        } else {
            Some(HoleInfo {
                addr: aligned_hole.addr + required_size,
                size: aligned_hole.size - required_size,
            })
        };

        Some(Allocation {
            info: HoleInfo {
                addr: aligned_hole.addr,
                size: required_size,
            },
            front_padding,
            back_padding,
        })
    }

    fn deallocate_from_hole_info(mut node: &mut FreeNode, mut hole: HoleInfo) {
        // These things are true of any allocation that we did
        assert!(hole.size >= size_of::<FreeNode>());
        assert!(hole.size == align_up(hole.size, align_of::<FreeNode>()));
        assert!(hole.addr == align_up(hole.addr, align_of::<FreeNode>()));

        loop {
            // Need to handle the special zero sized node, which is part of the free list object
            // and which we don't move about
            let node_addr = if node.size == 0 {
                0
            } else {
                node as *mut _ as usize
            };

            assert!(node_addr + node.size <= hole.addr, "Invalid deallocation");

            let next_node_info = node.next.as_ref().map(|next| next.info());
            match next_node_info {
                Some(next)
                    if node_addr + node.size == hole.addr && hole.addr + hole.size == next.addr =>
                {
                    // The free space fits exactly between this node and the next
                    node.size += hole.size + next.size;
                    node.next = node.next.as_mut().unwrap().next.take();
                }
                _ if node_addr + node.size == hole.addr => {
                    // The free space is directly after this node
                    node.size += hole.size;
                }
                Some(next) if hole.addr + hole.size == next.addr => {
                    // Immediateley before the next node, but not immediately after this one, so remove the next
                    // node, and add its size to the space we're deallocating
                    node.next = node.next.as_mut().unwrap().next.take();
                    hole.size += next.size;
                    continue;
                }
                Some(next) if next.addr <= hole.addr => {
                    // Block is behind the next free node, so move on to that
                    node = node.next.as_mut().unwrap();
                    continue;
                }
                _ => {
                    // block is between this node and the next, or this is the last node
                    let new_node = FreeNode {
                        size: hole.size,
                        next: node.next.take(),
                    };
                    debug_assert_eq!(hole.addr % align_of::<FreeNode>(), 0);
                    let ptr = hole.addr as *mut FreeNode;
                    unsafe { ptr.write(new_node) };
                    node.next = Some(unsafe { &mut *ptr });
                }
            }
            break;
        }
    }
}

struct FreeNode {
    size: usize,
    next: Option<&'static mut FreeNode>,
}

impl FreeNode {
    pub fn info(&self) -> HoleInfo {
        HoleInfo {
            addr: (self as *const FreeNode) as usize,
            size: self.size,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    struct TestFreeList<'a> {
        storage_layout: AlignedLayout,
        storage: &'a mut [u8],
        aligned_storage: usize,
        free_list: FreeList,
    }

    impl<'a> Drop for TestFreeList<'a> {
        fn drop(&mut self) {
            unsafe { alloc::alloc::dealloc(self.storage.as_mut_ptr(), self.storage_layout.into()); }
        }
    }


    fn make_free_list<'a>(size: usize, align: usize) -> TestFreeList<'a> {
        // Overallocate the storage so that we can make sure that align is met, but no higher alignments are. This is because the
        // free list is smart enough to look at the actual alignment of the memory, not just the minimum alignment
        let storage_layout = Layout::from_size_align(size + align, align).ok().and_then(|layout| FreeList::align_layout(layout)).expect("Invalid layout");
        let ptr = unsafe { alloc::alloc::alloc(*storage_layout.layout()) };
        assert_ne!(ptr, core::ptr::null_mut());
        let storage = unsafe { alloc::slice::from_raw_parts_mut(ptr, storage_layout.size()) };
        let aligned_storage = if storage.as_mut_ptr() as usize & ((2 * storage_layout.align()) - 1) == 0 {
            storage.as_mut_ptr() as usize + storage_layout.align()
        } else {
            storage.as_mut_ptr() as usize
        };

        // Only tell the free list about the size we were actually asked for
        let free_list = unsafe { FreeList::new(aligned_storage, aligned_storage + size) };
        
        TestFreeList {
            storage_layout,
            storage,
            aligned_storage,
            free_list,
        }
    }

    #[test_case]
    fn empty_free_list() {
        let t = make_free_list(FreeList::min_alloc_size(), FreeList::min_alignment());

        // Verify that the free space is all available
        assert_eq!(t.free_list.free_space(), FreeList::min_alloc_size());
        assert_eq!(t.free_list.allocated_space(), 0);
        assert_eq!(t.free_list.node_count(), 1);
    }

    #[test_case]
    fn small_allocations() {
        let mut t = make_free_list(FreeList::min_alloc_size(), FreeList::min_alignment());

        let mut align = 1;
        while align <= FreeList::min_alignment() {
            for size in 0..=FreeList::min_alloc_size() {
                let layout = Layout::from_size_align(size, align).unwrap();
                let layout = FreeList::align_layout(layout).unwrap();
                assert_eq!(layout.size(), FreeList::min_alloc_size());
                assert_eq!(layout.align(), FreeList::min_alignment());

                let allocation = t.free_list.allocate(layout);
                assert!(allocation.is_some());
                assert_eq!(allocation.unwrap().as_ptr() as usize, t.aligned_storage);
                assert_eq!(t.free_list.free_space(), 0);
                assert_eq!(t.free_list.allocated_space(), FreeList::min_alloc_size());
                assert_eq!(t.free_list.node_count(), 0);

                t.free_list.deallocate(allocation.unwrap(), layout);
                assert_eq!(t.free_list.free_space(), FreeList::min_alloc_size());
                assert_eq!(t.free_list.allocated_space(), 0);
                assert_eq!(t.free_list.node_count(), 1);
            }

            // Do an oversized allocation at the good alignment
            let layout = Layout::from_size_align(FreeList::min_alloc_size() + 1, align).unwrap();
            let layout = FreeList::align_layout(layout).unwrap();
            assert_eq!(layout.size(), FreeList::min_alloc_size() + FreeList::min_alignment());
            assert_eq!(layout.align(), FreeList::min_alignment());

            let allocation = t.free_list.allocate(layout);
            assert!(allocation.is_none());

            align *= 2;
        }

        // Do an overaligned allocation at a good size.
        let layout = Layout::from_size_align(1, FreeList::min_alignment() * 2).unwrap();
        let layout = FreeList::align_layout(layout).unwrap();
        assert_eq!(layout.size(), FreeList::min_alloc_size());
        assert_eq!(layout.align(), FreeList::min_alignment() * 2);

        let allocation = t.free_list.allocate(layout);
        assert!(allocation.is_none());
    }

    #[test_case]
    fn test_overalignment() {
        // Allocate a big free list so we have room to do bigger alignments up to 8K
        let mut t = make_free_list(16384, FreeList::min_alignment());

        let mut align = FreeList::min_alignment() * 2;
        while align <= 8192 {
            let layout = Layout::from_size_align(1, align).unwrap();
            let layout = FreeList::align_layout(layout).unwrap();
            assert_eq!(layout.size(), FreeList::min_alloc_size());
            assert_eq!(layout.align(), align);

            let allocation = t.free_list.allocate(layout);
            assert!(allocation.is_some());
            assert_eq!(allocation.unwrap().as_ptr() as usize & (align - 1), 0);
            assert_eq!(t.free_list.free_space(), 16384 - FreeList::min_alloc_size());
            assert_eq!(t.free_list.allocated_space(), FreeList::min_alloc_size());
            // There should be two free nodes - one before and one after the allocation
            assert_eq!(t.free_list.node_count(), 2);

            t.free_list.deallocate(allocation.unwrap(), layout);
            assert_eq!(t.free_list.free_space(), 16384);
            assert_eq!(t.free_list.allocated_space(), 0);
            assert_eq!(t.free_list.node_count(), 1);

            align *= 2;
        }
    }
}
