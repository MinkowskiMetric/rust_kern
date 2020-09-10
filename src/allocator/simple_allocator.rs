use crate::paging::{allocate_region, Region, RegionFlags, PAGE_SIZE};
use core::alloc::{GlobalAlloc, Layout};
use core::mem::{align_of, size_of, MaybeUninit};
use core::ops::{Deref, DerefMut};
use core::ptr::{null_mut, NonNull};
use spin::Mutex;

const MINIMUM_HEAP_REGION_PAGES: usize = 16;
const MINIMUM_HEAP_REGION_SIZE: usize = (MINIMUM_HEAP_REGION_PAGES * PAGE_SIZE as usize);

fn align_down(addr: usize, align: usize) -> usize {
    if align.is_power_of_two() {
        addr & !(align - 1)
    } else if align == 0 {
        addr
    } else {
        panic!("`align` must be a power of 2");
    }
}

/// Align upwards. Returns the smallest x with alignment `align`
/// so that x >= addr. The alignment must be a power of 2.
fn align_up(addr: usize, align: usize) -> usize {
    align_down(addr + align - 1, align)
}

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

struct FreeList {
    head: FreeNode,
}

impl FreeList {
    pub const fn empty() -> Self {
        Self {
            head: FreeNode {
                size: 0,
                next: None,
            },
        }
    }

    pub unsafe fn new(start: u64, limit: u64) -> Self {
        // This should be a no-op since the allocation should come from the page
        // allocator and be page aligned, but it does not hurt to be safe
        let aligned_start = align_up(start as usize, align_of::<FreeNode>()) as u64;
        // And we should definitely be able to fit a free node in the list
        let size = limit.saturating_sub(aligned_start) as usize;
        assert!(size >= size_of::<FreeNode>());

        let ptr = aligned_start as *mut FreeNode;
        ptr.write(FreeNode { size, next: None });

        FreeList {
            head: FreeNode {
                size: 0,
                next: Some(&mut *ptr),
            },
        }
    }

    pub fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        Self::tail_allocate(&mut self.head, layout).map(|allocation| {
            if let Some(front_padding) = allocation.front_padding {
                Self::deallocate_from_hole_info(&mut self.head, front_padding);
            }

            if let Some(back_padding) = allocation.back_padding {
                Self::deallocate_from_hole_info(&mut self.head, back_padding);
            }

            NonNull::new(allocation.info.addr as *mut u8).unwrap()
        })
    }

    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        Self::deallocate_from_hole_info(
            &mut self.head,
            HoleInfo {
                addr: ptr.as_ptr() as usize,
                size: layout.size(),
            },
        );
    }

    fn tail_allocate(mut prev_node: &mut FreeNode, layout: Layout) -> Option<Allocation> {
        loop {
            let allocation = prev_node
                .next
                .as_mut()
                .and_then(|current| Self::allocate_from_hole_info(current.info(), layout));
            if let Some(allocation) = allocation {
                // So, remove the free node from the list and return the allocation descriptor
                prev_node.next = prev_node.next.as_mut().unwrap().next.take();
                return Some(allocation);
            } else if prev_node.next.is_some() {
                prev_node = prev_node.next.as_mut().unwrap();
            } else {
                // Out of memory
                return None;
            }
        }
    }

    fn allocate_from_hole_info(hole: HoleInfo, layout: Layout) -> Option<Allocation> {
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

struct HeapRegionList {
    head: Option<&'static mut HeapRegion>,
}

impl HeapRegionList {
    pub const fn empty() -> Self {
        Self { head: None }
    }

    pub fn new(region: Region) -> Self {
        let (start, limit) = (region.start(), region.limit());

        // This should be a no-op since the allocation should come from the page
        // allocator and be page aligned, but it does not hurt to be safe
        let aligned_start = align_up(start as usize, align_of::<HeapRegion>()) as u64;
        // And we should definitely be able to fit a free node in the list
        let size = limit.saturating_sub(aligned_start) as usize;
        assert!(size >= size_of::<HeapRegion>());

        let ptr = aligned_start as *mut HeapRegion;
        unsafe {
            ptr.write(HeapRegion {
                alloc_region: region,
                free_list: unsafe {
                    FreeList::new(aligned_start + size_of::<HeapRegion>() as u64, limit)
                },
                next: None,
            })
        };

        HeapRegionList {
            head: Some(unsafe { &mut *ptr }),
        }
    }

    fn align_layout(layout: Layout) -> Option<Layout> {
        // Fixing up the layout in here is useful because we do it before allocation and deallocation,
        // which can simplify things. It makes life a lot easier if the minimal alignment is the same
        // as our free node, and that the size makes sure the end of the allocation is aligned. We also
        // avoid allocating anything smaller than our free node,
        let required_alignment = layout.align();
        let required_alignment = required_alignment.max(align_of::<FreeNode>());

        let required_size = layout.size();
        let required_size = align_up(
            required_size.max(size_of::<FreeNode>()),
            align_of::<FreeNode>(),
        );

        let ret = Layout::from_size_align(required_size, required_alignment).ok();
        use crate::println;
        println!("Original layout: {:?}", layout);
        println!("Fixed up layout: {:?}", ret);
        ret
    }

    pub unsafe fn alloc(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        Self::align_layout(layout).and_then(|aligned_layout| {
            self.walk_regions(|region| region.allocate(aligned_layout))
                .or_else(|| self.expand_and_allocate(aligned_layout))
        })
    }

    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) {
        Self::align_layout(layout)
            .and_then(|aligned_layout| {
                self.walk_regions(|region| region.deallocate(ptr, aligned_layout))
            })
            .expect("Failed to deallocate pointer");
    }

    fn walk_regions<T, F: Fn(&mut HeapRegion) -> Option<T>>(&mut self, f: F) -> Option<T> {
        self.head.as_mut().and_then(|mut this_region| loop {
            if let Some(v) = f(this_region) {
                return Some(v);
            } else if this_region.next.is_some() {
                this_region = this_region.next.as_mut().unwrap();
            } else {
                return None;
            }
        })
    }

    unsafe fn expand_and_allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        // The smallest possible region that this could fit in is the size of a region
        // header, plus whatever padding needed to get to alignment, plus the size of the
        // allocation, so let's work that out.

        // What ever we do, the minimum alignment is that of a free node, so we may always
        // need to insert some padding after the region header. We account for that here
        let header_size = align_up(size_of::<HeapRegion>(), align_of::<FreeNode>());

        let front_padding_size = if header_size == align_up(header_size, layout.align()) {
            // There doesn't need to be a padding node
            0
        } else {
            // We need to make sure that the padding node is large enough to accomodate the free node
            // header
            align_up(header_size + size_of::<FreeNode>(), layout.align()) - header_size
        };

        let required_size = header_size + front_padding_size + layout.size();
        let back_padding_size = if header_size == align_up(required_size, PAGE_SIZE as usize) {
            // The alignment is perfectly page aligned
            0
        } else {
            align_up(required_size + size_of::<FreeNode>(), PAGE_SIZE as usize) - required_size
        };

        // We don't need to worry about increasing the padding here. It guarantees page alignment,
        // and a free node already fits.
        let allocation_size = (required_size + back_padding_size).max(MINIMUM_HEAP_REGION_SIZE);
        let allocation_pages = allocation_size / PAGE_SIZE as usize;

        allocate_region(allocation_pages, RegionFlags::empty())
            .ok()
            .map(|region| {
                let mut new_region_list = HeapRegionList::new(region);

                new_region_list.head.as_mut().unwrap().next = self.head.take();
                self.head = new_region_list.head;

                self.head
                    .as_mut()
                    .unwrap()
                    .allocate(layout)
                    .expect("Couldn't make allocation from new region")
            })
    }
}

struct HeapRegion {
    alloc_region: Region,
    free_list: FreeList,
    next: Option<&'static mut HeapRegion>,
}

impl HeapRegion {
    pub fn allocate(&mut self, layout: Layout) -> Option<NonNull<u8>> {
        self.free_list.allocate(layout)
    }

    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: Layout) -> Option<()> {
        if self.contains(ptr, layout.size()) {
            self.free_list.deallocate(ptr, layout);
            Some(())
        } else {
            None
        }
    }

    pub fn contains(&self, ptr: NonNull<u8>, size: usize) -> bool {
        todo!("Implement contains")
    }
}

pub struct SimpleAllocator {
    head_region: Mutex<HeapRegionList>,
}

impl SimpleAllocator {
    pub fn new() -> Self {
        Self {
            head_region: Mutex::new(HeapRegionList::empty()),
        }
    }
}

unsafe impl GlobalAlloc for SimpleAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.head_region
            .lock()
            .alloc(layout)
            .map_or(null_mut(), |n| n.as_ptr())
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        panic!("Allocator not implemented");
    }
}
