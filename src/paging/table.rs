use super::page_entry::{PresentPageFlags, RawPresentPte, RawPte};
use super::{map_page, HyperspaceMapping, MemoryError, Result};
use crate::physmem;
use crate::physmem::Frame;
use bootloader::BootInfo;
use core::convert::{Infallible, TryFrom};
use core::fmt;
use core::marker::PhantomData;
use core::num::TryFromIntError;
use core::ops::{Deref, DerefMut};
use core::ops::{Index, IndexMut};

const ENTRY_COUNT: u16 = 512;

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
pub struct PageTable<L: PageTableLevel>([RawPte; ENTRY_COUNT as usize], PhantomData<L>);

impl<L: PageTableLevel> PageTable<L> {
    pub unsafe fn at_virtual_address(addr: u64) -> &'static Self {
        &*(addr as *const Self)
    }

    pub unsafe fn at_virtual_address_mut(addr: u64) -> &'static mut Self {
        &mut *(addr as *mut Self)
    }

    pub fn iter<'a>(&'a self) -> core::slice::Iter<'a, RawPte> {
        self.0.iter()
    }

    pub fn iter_mut<'a>(&'a mut self) -> core::slice::IterMut<'a, RawPte> {
        self.0.iter_mut()
    }

    pub fn zero(&mut self) {
        for entry in self.iter_mut() {
            *entry = RawPte::unused();
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
                    .present()
                    .map(|present_pte| present_pte.is_huge())
                    .unwrap_or(false),
                "Huge page not supported"
            );
            let new_page_table = physmem::allocate_frame()
                .expect("Failed to allocate frame in boot_create_next_table");
            self[index] = RawPresentPte::from_frame_and_flags(
                new_page_table,
                PresentPageFlags::WRITABLE | PresentPageFlags::USER_ACCESSIBLE,
            )
            .into();
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
                    .present()
                    .map(|present_pte| present_pte.is_huge())
                    .unwrap_or(false),
                "Huge page not supported"
            );
            let new_page_table = physmem::allocate_frame()
                .expect("Failed to allocate frame in boot_create_next_table");
            self[index] = RawPresentPte::from_frame_and_flags(
                new_page_table,
                PresentPageFlags::WRITABLE | PresentPageFlags::USER_ACCESSIBLE,
            )
            .into();
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
        self[index]
            .present()
            .map(|present_pte| present_pte.frame())
            .or(Err(MemoryError::NotMapped))
    }
}

impl<L: PageTableLevel> Index<PageTableIndex> for PageTable<L> {
    type Output = RawPte;

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
