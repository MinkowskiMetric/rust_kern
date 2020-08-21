use bootloader::BootInfo;

mod frame;
mod frame_allocator;
mod page;
mod page_table;

use crate::println;
pub use page_table::{PageOffset, PageTableIndex};

use frame_allocator::{FrameAllocator, FrameDeallocator};

pub fn early_init(boot_info: &'static BootInfo) {
    let mut early_frame_allocator =
        unsafe { frame_allocator::EarlyFrameAllocator::from_boot_info(boot_info) };

    let f1 = early_frame_allocator.allocate_frame();
    println!("Allocated a frame {:?}", f1);
    if let Some(f1) = f1 {
        unsafe { early_frame_allocator.deallocate_frame(f1) };
        println!("Deallocated it");
    }

    println!(
        "Allocated a frame {:?}",
        early_frame_allocator.allocate_frame()
    );
    println!(
        "Allocated a frame {:?}",
        early_frame_allocator.allocate_frame()
    );
    println!(
        "Allocated a frame {:?}",
        early_frame_allocator.allocate_frame()
    );

    panic!("the streets of carlisle")
}
