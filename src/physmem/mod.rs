use crate::init_mutex::InitMutex;
use bootloader::{bootinfo::MemoryRegionType, BootInfo};
use bump::BumpAllocator;
use core::fmt;

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

pub fn page_align_down(addr: u64) -> u64 {
    addr & !(PAGE_SIZE - 1)
}

pub fn page_align_up(addr: u64) -> u64 {
    page_align_down(addr + PAGE_SIZE - 1)
}

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

struct LeakAllocator<SrcAllocator: FrameAllocator> {
    src_allocator: SrcAllocator,
}

impl<SrcAllocator: FrameAllocator> LeakAllocator<SrcAllocator> {
    pub fn new(src_allocator: SrcAllocator) -> Self {
        Self { src_allocator }
    }
}

impl<SrcAllocator: FrameAllocator> FrameAllocator for LeakAllocator<SrcAllocator> {
    fn free_frames(&self) -> usize {
        self.src_allocator.free_frames()
    }

    fn used_frames(&self) -> usize {
        self.src_allocator.used_frames()
    }

    fn allocate_frame(&mut self) -> Option<Frame> {
        self.src_allocator.allocate_frame()
    }

    fn deallocate_frame(&mut self, frame: Frame) {
        // do nothing and leak the frame
        use crate::println;
        println!("LEAKING PAGE {:?}", frame);
    }
}

static ALLOCATOR: InitMutex<LeakAllocator<BumpAllocator>> = InitMutex::new();

pub unsafe fn init(boot_info: &BootInfo) {
    let mut mem_position = 0;

    // We make multiple passes over the memory map. We place the immediately usable memory on the memory map first
    // then we put the stuff we can reclaim after
    for memory_region in boot_info.memory_map.iter() {
        if memory_region.region_type == MemoryRegionType::Usable {
            if mem_position >= MAX_MEMORY_AREAS {
                break;
            }
            MEMORY_MAP[mem_position] = MemoryArea {
                start: memory_region.range.start_addr(),
                limit: memory_region.range.end_addr(),
                mem_type: MemoryAreaType::Usable,
            };
            mem_position += 1;
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
                break;
            }
            MEMORY_MAP[mem_position] = MemoryArea {
                start: memory_region.range.start_addr(),
                limit: memory_region.range.end_addr(),
                mem_type: MemoryAreaType::Reclaimable,
            };
            mem_position += 1;
        }
    }

    ALLOCATOR.init(LeakAllocator::new(BumpAllocator::new(
        MemoryMapIterator::new(MemoryAreaType::Usable),
    )));
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Frame(u64);

impl Frame {
    pub fn containing_address(addr: u64) -> Self {
        Self(page_align_down(addr) / PAGE_SIZE)
    }

    pub fn index(&self) -> u64 {
        self.0
    }

    pub fn physical_address(&self) -> u64 {
        self.index() * PAGE_SIZE
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!("Frame({:#x})", self.physical_address()))
    }
}

pub fn free_frames() -> usize {
    ALLOCATOR.lock().free_frames()
}

pub fn used_frames() -> usize {
    ALLOCATOR.lock().used_frames()
}

pub fn allocate_frame() -> Option<Frame> {
    ALLOCATOR.lock().allocate_frame()
}

pub fn deallocate_frame(frame: Frame) {
    ALLOCATOR.lock().deallocate_frame(frame)
}

pub trait FrameAllocator {
    fn free_frames(&self) -> usize;
    fn used_frames(&self) -> usize;

    fn allocate_frame(&mut self) -> Option<Frame>;
    fn deallocate_frame(&mut self, frame: Frame);
}
