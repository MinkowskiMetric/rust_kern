use super::{
    align_up,
    free_list::{AlignedLayout, FreeList},
};
use crate::paging::{allocate_region, Region, PAGE_SIZE};
use core::alloc::{GlobalAlloc, Layout};
use core::mem::{align_of, size_of};
use core::ptr::{null_mut, NonNull};
use spin::Mutex;

const MINIMUM_HEAP_REGION_PAGES: usize = 16;
const MINIMUM_HEAP_REGION_SIZE: usize = MINIMUM_HEAP_REGION_PAGES * PAGE_SIZE;

// When we have an empty region, we don't release it back if our free space is less than this
const HEAP_RESERVE_LIMIT: usize = 128; // * 1024;

struct HeapRegionList {
    head: HeapRegion,
}

impl HeapRegionList {
    pub fn empty() -> Self {
        Self {
            head: HeapRegion {
                payload: None,
                next: None,
            },
        }
    }

    pub unsafe fn alloc(&mut self, original_layout: Layout) -> Option<NonNull<u8>> {
        FreeList::align_layout(original_layout).and_then(|aligned_layout| {
            Self::do_allocate(&mut self.head, aligned_layout)
                .or_else(|| self.expand_and_allocate(aligned_layout))
        })
    }

    unsafe fn do_allocate(
        mut prev_region: &mut HeapRegion,
        layout: AlignedLayout,
    ) -> Option<NonNull<u8>> {
        loop {
            let allocation = prev_region
                .next
                .as_mut()
                .and_then(|this_region| this_region.allocate(layout));
            if let Some(allocation) = allocation {
                return Some(allocation);
            } else if prev_region.next.is_some() {
                prev_region = prev_region.next.as_mut().unwrap();
            } else {
                // No allocation was found
                return None;
            }
        }
    }

    pub unsafe fn deallocate(&mut self, ptr: NonNull<u8>, original_layout: Layout) {
        FreeList::align_layout(original_layout).map(|aligned_layout| {
            if let Some(mut removed_region_list) =
                Self::do_deallocate(&mut self.head, ptr, aligned_layout)
            {
                let (removed_region_can_free, removed_region_free_space) = {
                    let removed_region = removed_region_list.next.as_ref().unwrap();
                    (removed_region.can_free(), removed_region.free_space())
                };

                // If we have enough free space, then we do not need to keep this region around and we can drop it.
                // But, we don't want to keep really big regions around, so if the regions free space is larger than
                // the default space we always drop it
                if !removed_region_can_free
                    || (removed_region_free_space < MINIMUM_HEAP_REGION_SIZE
                        && self.free_space() < HEAP_RESERVE_LIMIT)
                {
                    removed_region_list.next.as_mut().unwrap().next = self.head.next.take();
                    self.head.next = removed_region_list.next.take();
                } else {
                    // We have to move the payload out of the region that it is held in before we drop it, otherwise we end up with
                    // the memory going away part way through the drop which is weird.
                    core::mem::drop((removed_region_list.next.unwrap() as *mut HeapRegion).read());
                }
            }
        });
    }

    unsafe fn do_deallocate(
        mut prev_region: &mut HeapRegion,
        ptr: NonNull<u8>,
        layout: AlignedLayout,
    ) -> Option<HeapRegion> {
        loop {
            let deallocate_result = prev_region
                .next
                .as_mut()
                .and_then(|this_region| this_region.deallocate(ptr, layout));
            if let Some(_) = deallocate_result {
                if prev_region.next.as_ref().unwrap().allocated_space() == 0 {
                    let mut removed_region = HeapRegion {
                        payload: None,
                        next: prev_region.next.take(),
                    };
                    prev_region.next = removed_region.next.as_mut().unwrap().next.take();

                    return Some(removed_region);
                } else {
                    return None;
                }
            } else if prev_region.next.is_some() {
                prev_region = prev_region.next.as_mut().unwrap();
            } else {
                panic!("Failed to deallocate pointer {:#x}", ptr.as_ptr() as usize);
            }
        }
    }

    pub fn free_space(&self) -> usize {
        let mut prev_region = &self.head;
        let mut free_space = 0;
        loop {
            free_space += prev_region
                .next
                .as_ref()
                .map(|region| region.free_space())
                .unwrap_or(0);
            if prev_region.next.is_some() {
                prev_region = prev_region.next.as_ref().unwrap();
            } else {
                return free_space;
            }
        }
    }

    pub fn allocated_space(&self) -> usize {
        let mut prev_region = &self.head;
        let mut allocated_space = 0;
        loop {
            allocated_space += prev_region
                .next
                .as_ref()
                .map(|region| region.allocated_space())
                .unwrap_or(0);
            if prev_region.next.is_some() {
                prev_region = prev_region.next.as_ref().unwrap();
            } else {
                return allocated_space;
            }
        }
    }

    unsafe fn expand_and_allocate(&mut self, layout: AlignedLayout) -> Option<NonNull<u8>> {
        // The smallest possible region that this could fit in is the size of a region
        // header, plus whatever padding needed to get to alignment, plus the size of the
        // allocation, so let's work that out.

        // What ever we do, the minimum alignment is that of a free node, so we may always
        // need to insert some padding after the region header. We account for that here
        let header_size = align_up(size_of::<HeapRegion>(), FreeList::min_alignment());

        let front_padding_size = if header_size == align_up(header_size, layout.align()) {
            // There doesn't need to be a padding node
            0
        } else {
            // We need to make sure that the padding node is large enough to accomodate the free node
            // header
            align_up(header_size + FreeList::min_alloc_size(), layout.align()) - header_size
        };

        let required_size = header_size + front_padding_size + layout.size();
        let back_padding_size = if header_size == align_up(required_size, PAGE_SIZE) {
            // The alignment is perfectly page aligned
            0
        } else {
            align_up(required_size + FreeList::min_alloc_size(), PAGE_SIZE) - required_size
        };

        // We don't need to worry about increasing the padding here. It guarantees page alignment,
        // and a free node already fits.
        let allocation_size = (required_size + back_padding_size).max(MINIMUM_HEAP_REGION_SIZE);
        let allocation_pages = allocation_size / PAGE_SIZE;

        allocate_region(allocation_pages).ok().map(|region| {
            let (start, limit) = (region.start(), region.limit());

            // This should be a no-op since the allocation should come from the page
            // allocator and be page aligned, but it does not hurt to be safe
            let aligned_start = align_up(start, align_of::<HeapRegion>());
            // And we should definitely be able to fit a free node in the list
            let size = limit.saturating_sub(aligned_start);
            assert!(size >= size_of::<HeapRegion>());

            let ptr = aligned_start as *mut HeapRegion;
            ptr.write(HeapRegion {
                payload: Some(HeapRegionPayload {
                    alloc_region: PayloadRegionAlloc::from_region(region),
                    can_free: true,
                    free_list: FreeList::new(aligned_start + size_of::<HeapRegion>(), limit),
                }),
                next: self.head.next.take(),
            });

            self.head.next = Some(&mut *ptr);

            self.head
                .next
                .as_mut()
                .unwrap()
                .allocate(layout)
                .expect("Couldn't make allocation from new region")
        })
    }
}

enum PayloadRegionAlloc {
    Buffer(&'static mut [u8]),
    Region(Region),
}

impl PayloadRegionAlloc {
    pub fn from_region(region: Region) -> Self {
        Self::Region(region)
    }

    pub fn from_slice(slice: &'static mut [u8]) -> Self {
        Self::Buffer(slice)
    }

    pub fn contains(&self, ptr: NonNull<u8>, size: usize) -> bool {
        let addr = ptr.as_ptr() as usize;

        match self {
            Self::Buffer(buffer) => {
                let start = buffer.as_ptr() as usize;
                let limit = start + buffer.len();
                addr >= start && size <= (limit - addr)
            }

            Self::Region(region) => {
                addr >= region.start() && addr <= region.limit() && size <= (region.limit() - addr)
            }
        }
    }
}

struct HeapRegionPayload {
    alloc_region: PayloadRegionAlloc,
    can_free: bool,
    free_list: FreeList,
}

impl HeapRegionPayload {
    pub fn allocate(&mut self, layout: AlignedLayout) -> Option<NonNull<u8>> {
        self.free_list.allocate(layout)
    }

    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: AlignedLayout) -> Option<()> {
        if self.contains(ptr, layout.size()) {
            self.free_list.deallocate(ptr, layout);
            Some(())
        } else {
            None
        }
    }

    pub fn contains(&self, ptr: NonNull<u8>, size: usize) -> bool {
        self.alloc_region.contains(ptr, size)
    }

    pub fn free_space(&self) -> usize {
        self.free_list.free_space()
    }

    pub fn allocated_space(&self) -> usize {
        self.free_list.allocated_space()
    }

    pub fn can_free(&self) -> bool {
        self.can_free
    }
}

struct HeapRegion {
    payload: Option<HeapRegionPayload>,
    next: Option<&'static mut HeapRegion>,
}

impl HeapRegion {
    pub fn allocate(&mut self, layout: AlignedLayout) -> Option<NonNull<u8>> {
        self.payload
            .as_mut()
            .and_then(|payload| payload.allocate(layout))
    }

    pub fn deallocate(&mut self, ptr: NonNull<u8>, layout: AlignedLayout) -> Option<()> {
        self.payload
            .as_mut()
            .and_then(|payload| payload.deallocate(ptr, layout))
    }

    pub fn free_space(&self) -> usize {
        self.payload
            .as_ref()
            .map(|payload| payload.free_space())
            .unwrap_or(0)
    }

    pub fn allocated_space(&self) -> usize {
        self.payload
            .as_ref()
            .map(|payload| payload.allocated_space())
            .unwrap_or(0)
    }

    pub fn can_free(&self) -> bool {
        self.payload
            .as_ref()
            .map(|payload| payload.can_free())
            .unwrap_or(false)
    }
}

pub struct SimpleAllocator {
    head_region: Mutex<HeapRegionList>,
}

impl SimpleAllocator {
    pub fn new() -> Self {
        use core::sync::atomic::{AtomicBool, Ordering};

        static INITIALIZED: AtomicBool = AtomicBool::new(false);

        if INITIALIZED.swap(true, Ordering::SeqCst) {
            // Already initialized. This is certainly an unusual case, but we can easily cover it
            // by simply creating a new empty heap.
            Self {
                head_region: Mutex::new(HeapRegionList::empty()),
            }
        } else {
            const INITIAL_HEAP_REGION_SIZE: usize = 128 * 1024;

            #[repr(align(4096))]
            #[repr(C)]
            struct InitialHeapBuffer([u8; INITIAL_HEAP_REGION_SIZE]);

            // We set this up so that it is in the BSS section so we hopefully don't need to load it off the disk
            static mut INITIAL_HEAP_REGION: InitialHeapBuffer =
                InitialHeapBuffer([0; INITIAL_HEAP_REGION_SIZE]);

            let region_start = unsafe { (&mut INITIAL_HEAP_REGION.0[0] as *mut u8) as usize };
            let region_end = region_start + INITIAL_HEAP_REGION_SIZE;

            let aligned_start = align_up(region_start, align_of::<HeapRegion>());
            let size = region_end.saturating_sub(aligned_start);
            assert!(size >= size_of::<HeapRegion>());

            let ptr = aligned_start as *mut HeapRegion;
            unsafe {
                ptr.write(HeapRegion {
                    payload: Some(HeapRegionPayload {
                        alloc_region: PayloadRegionAlloc::from_slice(&mut INITIAL_HEAP_REGION.0),
                        can_free: false,
                        free_list: FreeList::new(
                            aligned_start + size_of::<HeapRegion>(),
                            region_end,
                        ),
                    }),
                    next: None,
                })
            }

            Self {
                head_region: Mutex::new(HeapRegionList {
                    head: HeapRegion {
                        payload: None,
                        next: Some(unsafe { &mut *ptr }),
                    },
                }),
            }
        }
    }

    pub fn allocated_space(&self) -> usize {
        self.head_region.lock().allocated_space()
    }

    pub fn free_space(&self) -> usize {
        self.head_region.lock().free_space()
    }
}

unsafe impl GlobalAlloc for SimpleAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.head_region
            .lock()
            .alloc(layout)
            .map_or(null_mut(), |n| n.as_ptr())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.head_region
            .lock()
            .deallocate(NonNull::new(ptr).unwrap(), layout);
    }
}
