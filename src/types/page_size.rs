pub trait PageSize: Copy + Eq + PartialOrd + Ord {
    const SIZE: u64;
    const SIZE_AS_DEBUG_STR: &'static str;
}

pub trait NotGiantPageSize: PageSize {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Size4KiB {}

impl PageSize for Size4KiB {
    const SIZE: u64 = 4096;
    const SIZE_AS_DEBUG_STR: &'static str = "4KiB";
}

impl NotGiantPageSize for Size4KiB {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Size2MiB {}

impl PageSize for Size2MiB {
    const SIZE: u64 = Size4KiB::SIZE * 512;
    const SIZE_AS_DEBUG_STR: &'static str = "2MiB";
}

impl NotGiantPageSize for Size2MiB {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Size1GiB {}

impl PageSize for Size1GiB {
    const SIZE: u64 = Size2MiB::SIZE * 512;
    const SIZE_AS_DEBUG_STR: &'static str = "1GiB";
}
