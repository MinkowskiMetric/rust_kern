use crate::println;
use crate::types::PhysicalAddress;
use bootloader::{bootinfo::MemoryRegionType, BootInfo};
use bump::BumpAllocator;
use spin::Mutex;

mod bump;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum MemoryAreaType {
    Null = 0,
    Usable = 1,
    Reclaimable = 2,
}

#[repr(packed)]
#[derive(Copy, Clone, Debug)]
pub struct MemoryArea {
    pub start: u64,
    pub limit: u64,
    pub mem_type: MemoryAreaType,
}

pub const PAGE_SIZE: u64 = 4096;

const MAX_MEMORY_AREAS: usize = 64;

static mut MEMORY_MAP: [MemoryArea; MAX_MEMORY_AREAS] = [MemoryArea {
    start: 0,
    limit: 0,
    mem_type: MemoryAreaType::Null,
}; MAX_MEMORY_AREAS];

#[derive(Clone)]
pub struct MemoryMapIterator {
    match_type: MemoryAreaType,
    next_pos: usize,
}

impl MemoryMapIterator {
    pub fn new(match_type: MemoryAreaType) -> Self {
        Self {
            match_type,
            next_pos: 0,
        }
    }
}

impl Iterator for MemoryMapIterator {
    type Item = &'static MemoryArea;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next_pos < MAX_MEMORY_AREAS
            && unsafe { MEMORY_MAP[self.next_pos].mem_type } != self.match_type
        {
            self.next_pos += 1;
        }

        if self.next_pos >= MAX_MEMORY_AREAS {
            None
        } else {
            let this_pos = self.next_pos;
            self.next_pos += 1;
            unsafe { Some(&MEMORY_MAP[this_pos]) }
        }
    }
}

static ALLOCATOR: Mutex<Option<BumpAllocator>> = Mutex::new(None);

pub unsafe fn init(boot_info: &BootInfo) {
    let mut mem_position = 0;
    let mut total_available_memory = 0;
    let mut total_pending_memory = 0;
    let mut has_skipped_regions = false;

    // We make multiple passes over the memory map. We place the immediately usable memory on the memory map first
    // then we put the stuff we can reclaim after
    for memory_region in boot_info.memory_map.iter() {
        if memory_region.region_type == MemoryRegionType::Usable {
            if mem_position >= MAX_MEMORY_AREAS {
                has_skipped_regions = true;
                break;
            }
            MEMORY_MAP[mem_position] = MemoryArea {
                start: memory_region.range.start_addr(),
                limit: memory_region.range.end_addr(),
                mem_type: MemoryAreaType::Usable,
            };
            println!("{:?}", MEMORY_MAP[mem_position]);
            mem_position += 1;
            total_available_memory +=
                memory_region.range.end_addr() - memory_region.range.start_addr();
        }
    }

    for memory_region in boot_info.memory_map.iter() {
        if memory_region.region_type == MemoryRegionType::KernelStack
            || memory_region.region_type == MemoryRegionType::PageTable
            || memory_region.region_type == MemoryRegionType::Bootloader
            || memory_region.region_type == MemoryRegionType::BootInfo
            || memory_region.region_type == MemoryRegionType::Package
        {
            if mem_position >= MAX_MEMORY_AREAS {
                has_skipped_regions = true;
                break;
            }
            MEMORY_MAP[mem_position] = MemoryArea {
                start: memory_region.range.start_addr(),
                limit: memory_region.range.end_addr(),
                mem_type: MemoryAreaType::Reclaimable,
            };
            mem_position += 1;
            total_pending_memory +=
                memory_region.range.end_addr() - memory_region.range.start_addr();
        } else if memory_region.region_type != MemoryRegionType::Usable {
            println!("Ignoring memory region {:?}", memory_region);
        }
    }

    if has_skipped_regions {
        println!("Out of memory regions - some available regions were skipped");
    }

    println!("Total available memory: {} bytes", total_available_memory);
    println!("Total pending memory: {} bytes", total_pending_memory);

    *ALLOCATOR.lock() = Some(BumpAllocator::new(MemoryMapIterator::new(
        MemoryAreaType::Usable,
    )));
}

macro_rules! check_allocator {
    { |ref $alloc:ident| $code:block } => {
        if let Some(ref $alloc) = *ALLOCATOR.lock() {
            $code
        } else {
            panic!("frame allocator not initialized");
        }
    };

    { |ref mut $alloc:ident| $code:block } => {
        if let Some(ref mut $alloc) = *ALLOCATOR.lock() {
            $code
        } else {
            panic!("frame allocator not initialized");
        }
    }
}

pub fn free_frames() -> usize {
    check_allocator! { |ref allocator| { allocator.free_frames() } }
}

pub fn used_frames() -> usize {
    check_allocator! { |ref allocator| { allocator.used_frames() } }
}

pub fn allocate_frame() -> Option<PhysicalAddress> {
    check_allocator! { |ref mut allocator| { allocator.allocate_frame() } }
}

pub fn deallocate_frame(frame: PhysicalAddress) {
    check_allocator! { |ref mut allocator| { allocator.deallocate_frame(frame); } }
}

pub trait FrameAllocator {
    fn free_frames(&self) -> usize;
    fn used_frames(&self) -> usize;

    fn allocate_frame(&mut self) -> Option<PhysicalAddress>;
    fn deallocate_frame(&mut self, frame: PhysicalAddress);
}
