use super::{map_page, HyperspaceMapping, MemoryError, Result};
use crate::physmem;
use crate::physmem::Frame;
use bitflags::bitflags;
use bootloader::BootInfo;
use core::convert::{Infallible, TryFrom};
use core::fmt;
use core::marker::PhantomData;
use core::num::TryFromIntError;
use core::ops::{Deref, DerefMut};
use core::ops::{Index, IndexMut};

const ENTRY_COUNT: u16 = 512;

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
        const KERNEL_PROTECTED_PML4 = 1 << 9;
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

#[derive(Debug, Clone, Copy)]
pub struct InvalidPageTableIndex(());

impl From<Infallible> for InvalidPageTableIndex {
    fn from(_: Infallible) -> Self {
        Self(())
    }
}

impl From<TryFromIntError> for InvalidPageTableIndex {
    fn from(_: TryFromIntError) -> Self {
        Self(())
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PageTableIndex(u16);

impl PageTableIndex {
    pub const fn new_truncate(val: u16) -> Self {
        Self(val % ENTRY_COUNT)
    }

    pub const unsafe fn new_unchecked(val: u16) -> Self {
        Self(val)
    }
}

impl fmt::Debug for PageTableIndex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!("PageTableIndex({:#x})", self.0))
    }
}

macro_rules! page_table_index_conversions (
    () => { };
    ($t:ty $(, $others:ty)* $(,)?) => {
        page_table_index_conversions!($($others),*);

        impl TryFrom<$t> for PageTableIndex {
            type Error = InvalidPageTableIndex;

            fn try_from(val: $t) -> core::result::Result<Self, Self::Error> {
                let val = u16::try_from(val)?;

                if val < ENTRY_COUNT {
                    Ok(Self(val))
                } else {
                    Err(InvalidPageTableIndex(()))
                }
            }
        }

        impl From<PageTableIndex> for $t {
            fn from(val: PageTableIndex) -> Self {
                val.0.into()
            }
        }
    }
);

page_table_index_conversions!(u16, u32, u64, usize,);

#[derive(Debug, Clone, Copy)]
pub struct InvalidPageOffset(());

impl From<Infallible> for InvalidPageOffset {
    fn from(_: Infallible) -> Self {
        Self(())
    }
}

impl From<TryFromIntError> for InvalidPageOffset {
    fn from(_: TryFromIntError) -> Self {
        Self(())
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PageOffset(u16);

impl PageOffset {
    pub const fn new_truncate(val: u16) -> Self {
        Self(val % 4096)
    }

    pub const unsafe fn new_unchecked(val: u16) -> Self {
        Self(val)
    }
}

impl fmt::Debug for PageOffset {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!("PageOffset({:#x})", self.0))
    }
}

macro_rules! page_offset_conversions (
    () => { };
    ($t:ty $(, $others:ty)* $(,)?) => {
        page_offset_conversions!($($others),*);

        impl TryFrom<$t> for PageOffset {
            type Error = InvalidPageOffset;

            fn try_from(val: $t) -> core::result::Result<Self, Self::Error> {
                let val = u16::try_from(val)?;

                if val < 4096 {
                    Ok(Self(val))
                } else {
                    Err(InvalidPageOffset(()))
                }
            }
        }

        impl From<PageOffset> for $t {
            fn from(val: PageOffset) -> Self {
                val.0.into()
            }
        }
    }
);

page_offset_conversions!(u16, u32, u64, usize,);

pub const fn page_offset(va: u64) -> PageOffset {
    PageOffset::new_truncate(va as u16)
}

pub const fn p1_index(va: u64) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12) as u16)
}

pub const fn p2_index(va: u64) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12 >> 9) as u16)
}

pub const fn p3_index(va: u64) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12 >> 9 >> 9) as u16)
}

pub const fn p4_index(va: u64) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12 >> 9 >> 9 >> 9) as u16)
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    pub fn from_frame_and_flags(frame: Frame, flags: PageFlags) -> Self {
        Self(frame.physical_address() | flags.bits())
    }

    #[inline]
    pub const fn flags(&self) -> PageFlags {
        PageFlags::from_bits_truncate(self.0)
    }

    #[inline]
    pub fn frame(&self) -> Frame {
        Frame::containing_address(self.0 & 0x000fffff_fffff000)
    }

    pub fn is_unused(&self) -> bool {
        self.0 == 0
    }

    pub fn set_unused(&mut self) {
        self.0 = 0;
    }
}

pub trait PageTableLevel {}

pub trait HierarchyLevel: PageTableLevel {
    type NextLevel: PageTableLevel;
}

pub enum L4 {}
pub enum L3 {}
pub enum L2 {}
pub enum L1 {}

impl PageTableLevel for L4 {}
impl PageTableLevel for L3 {}
impl PageTableLevel for L2 {}
impl PageTableLevel for L1 {}

impl HierarchyLevel for L4 {
    type NextLevel = L3;
}

impl HierarchyLevel for L3 {
    type NextLevel = L2;
}

impl HierarchyLevel for L2 {
    type NextLevel = L1;
}

pub trait BootPageTable<L: PageTableLevel> {
    unsafe fn boot_create_next_table<'a>(
        &'a mut self,
        boot_info: &BootInfo,
        index: PageTableIndex,
    ) -> &'a mut PageTable<L::NextLevel>
    where
        L: HierarchyLevel;

    unsafe fn boot_next_table<'a>(
        &'a self,
        boot_info: &BootInfo,
        index: PageTableIndex,
    ) -> Result<&'a PageTable<L::NextLevel>>
    where
        L: HierarchyLevel;

    unsafe fn boot_next_table_mut<'a>(
        &'a mut self,
        boot_info: &BootInfo,
        index: PageTableIndex,
    ) -> Result<&'a mut PageTable<L::NextLevel>>
    where
        L: HierarchyLevel;
}

#[repr(C)]
#[repr(align(4096))]
pub struct PageTable<L: PageTableLevel>([PageTableEntry; ENTRY_COUNT as usize], PhantomData<L>);

impl<L: PageTableLevel> PageTable<L> {
    pub unsafe fn at_virtual_address(addr: u64) -> &'static Self {
        &*(addr as *const Self)
    }

    pub unsafe fn at_virtual_address_mut(addr: u64) -> &'static mut Self {
        &mut *(addr as *mut Self)
    }

    pub fn iter<'a>(&'a self) -> core::slice::Iter<'a, PageTableEntry> {
        self.0.iter()
    }

    pub fn iter_mut<'a>(&'a mut self) -> core::slice::IterMut<'a, PageTableEntry> {
        self.0.iter_mut()
    }

    pub fn zero(&mut self) {
        for entry in self.iter_mut() {
            entry.set_unused();
        }
    }
}

impl<L: 'static + HierarchyLevel> BootPageTable<L> for PageTable<L> {
    unsafe fn boot_create_next_table<'a>(
        &'a mut self,
        boot_info: &BootInfo,
        index: PageTableIndex,
    ) -> &'a mut PageTable<L::NextLevel> {
        if self.next_table_frame(index).is_err() {
            assert!(
                !self[index]
                    .flags()
                    .contains(PageFlags::PRESENT | PageFlags::HUGE_PAGE),
                "Huge page not supported"
            );
            assert!(
                !self[index]
                    .flags()
                    .contains(PageFlags::KERNEL_PROTECTED_PML4),
                "Allocating in unsafe kernel address space"
            );
            let new_page_table = physmem::allocate_frame()
                .expect("Failed to allocate frame in boot_create_next_table");
            self[index] = PageTableEntry::from_frame_and_flags(
                new_page_table,
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER_ACCESSIBLE,
            );
        }

        self.boot_next_table_mut(boot_info, index).unwrap()
    }

    unsafe fn boot_next_table<'a>(
        &'a self,
        boot_info: &BootInfo,
        index: PageTableIndex,
    ) -> Result<&'a PageTable<L::NextLevel>> {
        self.next_table_frame(index).map(|f| {
            PageTable::at_virtual_address(boot_info.physical_memory_offset + f.physical_address())
        })
    }

    unsafe fn boot_next_table_mut<'a>(
        &'a mut self,
        boot_info: &BootInfo,
        index: PageTableIndex,
    ) -> Result<&'a mut PageTable<L::NextLevel>> {
        self.next_table_frame(index).map(|f| {
            PageTable::at_virtual_address_mut(
                boot_info.physical_memory_offset + f.physical_address(),
            )
        })
    }
}

impl<L: 'static + HierarchyLevel> PageTable<L> {
    pub fn create_next_table(
        &mut self,
        index: PageTableIndex,
    ) -> Result<MappedPageTableMut<L::NextLevel>> {
        if self.next_table_frame(index) == Err(MemoryError::NotMapped) {
            assert!(
                !self[index]
                    .flags()
                    .contains(PageFlags::PRESENT | PageFlags::HUGE_PAGE),
                "Huge page not supported"
            );
            assert!(
                !self[index]
                    .flags()
                    .contains(PageFlags::KERNEL_PROTECTED_PML4),
                "Allocating in unsafe kernel address space"
            );
            let new_page_table = physmem::allocate_frame()
                .expect("Failed to allocate frame in boot_create_next_table");
            self[index] = PageTableEntry::from_frame_and_flags(
                new_page_table,
                PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::USER_ACCESSIBLE,
            );
        }

        self.next_table_mut(index)
    }

    pub fn next_table(&self, index: PageTableIndex) -> Result<MappedPageTable<L::NextLevel>> {
        self.next_table_frame(index)
            .and_then(|f| unsafe { MappedPageTable::from_frame(f) })
    }

    pub fn next_table_mut(
        &mut self,
        index: PageTableIndex,
    ) -> Result<MappedPageTableMut<L::NextLevel>> {
        self.next_table_frame(index)
            .and_then(|f| unsafe { MappedPageTableMut::from_frame(f) })
    }

    pub fn next_table_frame(&self, index: PageTableIndex) -> Result<Frame> {
        let entry = &self[index];
        if entry.flags().contains(PageFlags::PRESENT) {
            Ok(entry.frame())
        } else {
            Err(MemoryError::NotMapped)
        }
    }
}

impl<L: PageTableLevel> Index<PageTableIndex> for PageTable<L> {
    type Output = PageTableEntry;

    fn index(&self, index: PageTableIndex) -> &Self::Output {
        &self.0[usize::from(index)]
    }
}

impl<L: PageTableLevel> IndexMut<PageTableIndex> for PageTable<L> {
    fn index_mut(&mut self, index: PageTableIndex) -> &mut Self::Output {
        &mut self.0[usize::from(index)]
    }
}

pub struct MappedPageTable<L: PageTableLevel> {
    mapping: HyperspaceMapping,
    _marker: PhantomData<L>,
}

impl<L: PageTableLevel> MappedPageTable<L> {
    pub unsafe fn from_frame(frame: Frame) -> Result<Self> {
        map_page(frame).map(|mapping| Self {
            mapping,
            _marker: PhantomData,
        })
    }
}

impl<L: PageTableLevel> Deref for MappedPageTable<L> {
    type Target = PageTable<L>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mapping.as_ptr() }
    }
}

pub struct MappedPageTableMut<L: PageTableLevel> {
    mapping: HyperspaceMapping,
    _marker: PhantomData<L>,
}

impl<L: PageTableLevel> MappedPageTableMut<L> {
    pub unsafe fn from_frame(frame: Frame) -> Result<Self> {
        map_page(frame).map(|mapping| Self {
            mapping,
            _marker: PhantomData,
        })
    }
}

impl<L: PageTableLevel> Deref for MappedPageTableMut<L> {
    type Target = PageTable<L>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mapping.as_ptr() }
    }
}

impl<L: PageTableLevel> DerefMut for MappedPageTableMut<L> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mapping.as_mut_ptr() }
    }
}
