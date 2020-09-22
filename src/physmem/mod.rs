use bootloader::bootinfo::MemoryRegion;
use core::fmt;

mod frame_database;

pub const PAGE_SIZE: usize = 4096;

pub const fn page_align_down(addr: usize) -> usize {
    addr & !(PAGE_SIZE - 1)
}

pub const fn page_align_up(addr: usize) -> usize {
    page_align_down(addr + PAGE_SIZE - 1)
}

pub fn early_init<'a>(memory_map: impl IntoIterator<Item = &'a MemoryRegion>) {
    frame_database::early_init(memory_map);
}

pub fn init_post_paging<'a>(memory_map: impl IntoIterator<Item = &'a MemoryRegion> + Clone) {
    frame_database::init_post_paging(memory_map);
}

pub fn init_reclaim<'a>(memory_map: impl IntoIterator<Item = &'a MemoryRegion> + Clone) {
    frame_database::init_reclaim(memory_map);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Frame(usize);

impl Frame {
    pub fn containing_address(addr: usize) -> Self {
        Self(page_align_down(addr) / PAGE_SIZE)
    }

    pub fn from_index(index: usize) -> Self {
        Self(index)
    }

    pub fn index(&self) -> usize {
        self.0
    }

    pub fn physical_address(&self) -> usize {
        self.index() * PAGE_SIZE
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!("Frame({:#x})", self.physical_address()))
    }
}

pub fn free_frames() -> usize {
    frame_database::LOW_REGION.free_frames()
        + frame_database::NORMAL_REGION.free_frames()
        + frame_database::HIGH_REGION.free_frames()
}

pub fn used_frames() -> usize {
    frame_database::LOW_REGION.used_frames()
        + frame_database::NORMAL_REGION.used_frames()
        + frame_database::HIGH_REGION.used_frames()
}

pub fn allocate_kernel_frame() -> Option<Frame> {
    // For kernel allocations we do not try the high region because it isn't mapped and delivers frames
    // that are useless to the kernel
    frame_database::NORMAL_REGION
        .allocate_frame()
        .or_else(|| frame_database::LOW_REGION.allocate_frame())
}

pub fn allocate_user_frame() -> Option<Frame> {
    frame_database::HIGH_REGION
        .allocate_frame()
        .or_else(|| frame_database::NORMAL_REGION.allocate_frame())
        .or_else(|| frame_database::LOW_REGION.allocate_frame())
}

pub fn deallocate_frame(frame: Frame) {
    if frame_database::LOW_REGION.contains_frame(frame) {
        frame_database::LOW_REGION.deallocate_frame(frame)
    } else if frame_database::NORMAL_REGION.contains_frame(frame) {
        frame_database::NORMAL_REGION.deallocate_frame(frame)
    } else {
        frame_database::HIGH_REGION.deallocate_frame(frame)
    }
}

pub trait LockedFrameAllocator {
    fn free_frames(&self) -> usize;
    fn used_frames(&self) -> usize;

    fn allocate_frame(&mut self) -> Option<Frame>;
    fn deallocate_frame(&mut self, frame: Frame);

    fn contains_frame(&self, frame: Frame) -> bool;
}

pub trait FrameAllocator {
    fn free_frames(&self) -> usize;
    fn used_frames(&self) -> usize;

    fn allocate_frame(&self) -> Option<Frame>;
    fn deallocate_frame(&self, frame: Frame);

    fn contains_frame(&self, frame: Frame) -> bool;
}
