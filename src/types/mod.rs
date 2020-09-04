mod frame;
mod page;
mod page_size;
mod page_table;
mod phys;
mod virt;

pub use frame::Frame;
pub use page::Page;
pub use page_size::{NotGiantPageSize, PageSize, Size1GiB, Size2MiB, Size4KiB};
pub use page_table::{PageFlags, PageOffset, PageTableIndex};
pub use phys::{PhysicalAddress, PhysicalAddressNotValid};
pub use virt::{VirtualAddress, VirtualAddressNotValid};

fn align_down(addr: u64, align: u64) -> u64 {
    assert!(align.is_power_of_two(), "align must be power of two");
    addr & !(align - 1)
}

fn align_up(addr: u64, align: u64) -> u64 {
    align_down(addr + (align - 1), align)
}
