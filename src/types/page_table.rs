use crate::types::PhysicalAddress;
use bitflags::bitflags;
use core::{
    fmt,
    ops::{Index, IndexMut},
    slice::{Iter, IterMut, SliceIndex},
};

#[derive(Clone)]
#[repr(transparent)]
pub struct PageTableEntry {
    entry: u64,
}

impl PageTableEntry {
    pub const fn new() -> Self {
        Self { entry: 0 }
    }

    pub const fn is_unused(&self) -> bool {
        self.entry == 0
    }

    pub fn set_unused(&mut self) {
        self.entry = 0;
    }

    #[inline]
    pub const fn flags(&self) -> PageFlags {
        PageFlags::from_bits_truncate(self.entry)
    }

    #[inline]
    pub fn addr(&self) -> PhysicalAddress {
        PhysicalAddress::new(self.entry & 0x000fffff_fffff000)
    }
}

impl Default for PageTableEntry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut f = f.debug_struct("PageTableEntry");
        f.field("addr", &self.addr());
        f.field("flags", &self.flags());
        f.finish()
    }
}

bitflags! {
    pub struct PageFlags: u64 {
        /// Specifies whether the mapped frame or page table is loaded in memory.
        const PRESENT =         1;
        /// Controls whether writes to the mapped frames are allowed.
        ///
        /// If this bit is unset in a level 1 page table entry, the mapped frame is read-only.
        /// If this bit is unset in a higher level page table entry the complete range of mapped
        /// pages is read-only.
        const WRITABLE =        1 << 1;
        /// Controls whether accesses from userspace (i.e. ring 3) are permitted.
        const USER_ACCESSIBLE = 1 << 2;
        /// If this bit is set, a “write-through” policy is used for the cache, else a “write-back”
        /// policy is used.
        const WRITE_THROUGH =   1 << 3;
        /// Disables caching for the pointed entry is cacheable.
        const NO_CACHE =        1 << 4;
        /// Set by the CPU when the mapped frame or page table is accessed.
        const ACCESSED =        1 << 5;
        /// Set by the CPU on a write to the mapped frame.
        const DIRTY =           1 << 6;
        /// Specifies that the entry maps a huge frame instead of a page table. Only allowed in
        /// P2 or P3 tables.
        const HUGE_PAGE =       1 << 7;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 8;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_9 =           1 << 9;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_10 =          1 << 10;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_11 =          1 << 11;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_52 =          1 << 52;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_53 =          1 << 53;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_54 =          1 << 54;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_55 =          1 << 55;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_56 =          1 << 56;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_57 =          1 << 57;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_58 =          1 << 58;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_59 =          1 << 59;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_60 =          1 << 60;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_61 =          1 << 61;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_62 =          1 << 62;
        /// Forbid code execution from the mapped frames.
        ///
        /// Can be only used when the no-execute page protection feature is enabled in the EFER
        /// register.
        const NO_EXECUTE =      1 << 63;
    }
}

const ENTRY_COUNT: usize = 512;

/// Represents a page table.
///
/// Always page-sized.
///
/// This struct implements the `Index` and `IndexMut` traits, so the entries can be accessed
/// through index operations. For example, `page_table[15]` returns the 15th page table entry.
#[repr(align(4096))]
#[repr(C)]
pub struct PageTable {
    entries: [PageTableEntry; ENTRY_COUNT],
}

impl PageTable {
    pub const fn new() -> Self {
        PageTable {
            entries: [PageTableEntry::new(); ENTRY_COUNT],
        }
    }

    #[inline]
    pub fn zero(&mut self) {
        for entry in self {
            entry.set_unused();
        }
    }

    #[inline]
    pub fn iter<'a>(&'a self) -> Iter<'a, PageTableEntry> {
        self.entries.iter()
    }

    #[inline]
    pub fn iter_mut<'a>(&'a mut self) -> IterMut<'a, PageTableEntry> {
        self.entries.iter_mut()
    }
}

impl<'a> IntoIterator for &'a PageTable {
    type Item = &'a PageTableEntry;
    type IntoIter = Iter<'a, PageTableEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut PageTable {
    type Item = &'a mut PageTableEntry;
    type IntoIter = IterMut<'a, PageTableEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

impl<Idx: SliceIndex<[PageTableEntry]>> Index<Idx> for PageTable {
    type Output = Idx::Output;

    fn index(&self, index: Idx) -> &Self::Output {
        &self.entries[index]
    }
}

impl<Idx: SliceIndex<[PageTableEntry]>> IndexMut<Idx> for PageTable {
    fn index_mut(&mut self, index: Idx) -> &mut Self::Output {
        &mut self.entries[index]
    }
}

impl Index<PageTableIndex> for PageTable {
    type Output = PageTableEntry;

    fn index(&self, index: PageTableIndex) -> &Self::Output {
        &self.entries[usize::from(index)]
    }
}

impl IndexMut<PageTableIndex> for PageTable {
    fn index_mut(&mut self, index: PageTableIndex) -> &mut Self::Output {
        &mut self.entries[usize::from(index)]
    }
}

pub struct PageTableIndex(u16);

impl PageTableIndex {
    pub const fn new_truncate(index: u16) -> Self {
        Self(index % (ENTRY_COUNT as u16))
    }

    pub fn new(index: impl Into<u16>) -> Self {
        let value = index.into();
        assert!(value < (ENTRY_COUNT as u16));
        Self(value)
    }
}

impl From<PageTableIndex> for u16 {
    fn from(index: PageTableIndex) -> Self {
        index.0.into()
    }
}

impl From<PageTableIndex> for u32 {
    fn from(index: PageTableIndex) -> Self {
        index.0.into()
    }
}

impl From<PageTableIndex> for u64 {
    fn from(index: PageTableIndex) -> Self {
        index.0.into()
    }
}

impl From<PageTableIndex> for usize {
    fn from(index: PageTableIndex) -> Self {
        index.0.into()
    }
}

pub struct PageOffset(u16);

impl PageOffset {
    pub const fn new_truncate(offset: u16) -> Self {
        Self(offset % (1 << 12))
    }

    pub fn new(offset: u16) -> Self {
        assert!(offset < (1 << 12));
        Self(offset)
    }
}

impl From<PageOffset> for u16 {
    fn from(offset: PageOffset) -> Self {
        offset.0.into()
    }
}

impl From<PageOffset> for u32 {
    fn from(offset: PageOffset) -> Self {
        offset.0.into()
    }
}

impl From<PageOffset> for u64 {
    fn from(offset: PageOffset) -> Self {
        offset.0.into()
    }
}

impl From<PageOffset> for usize {
    fn from(offset: PageOffset) -> Self {
        offset.0.into()
    }
}
