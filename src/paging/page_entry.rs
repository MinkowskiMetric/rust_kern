use crate::physmem::Frame;
use bitflags::bitflags;
use core::convert::{TryFrom, TryInto};
use core::fmt;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::FromPrimitive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPteError(pub RawPte);

bitflags! {
    pub struct RawPageFlags: u64 {
        /// Specifies whether the mapped frame or page table is loaded in memory.
        const PRESENT = 1;
    }
}

// This is a PTE in it's rawest form as understood by the hardware. We do not infer
// any meaning on to it at all.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RawPte(u64);

impl RawPte {
    pub fn unused() -> Self {
        Self(0)
    }

    pub fn is_unused(&self) -> bool {
        *self == Self::unused()
    }

    pub fn flags(&self) -> RawPageFlags {
        RawPageFlags::from_bits_truncate(self.0)
    }

    pub fn is_present(&self) -> bool {
        self.flags().contains(RawPageFlags::PRESENT)
    }

    pub fn present(self) -> core::result::Result<RawPresentPte, InvalidPteError> {
        self.try_into()
    }

    pub fn not_present(self) -> core::result::Result<RawNotPresentPte, InvalidPteError> {
        self.try_into()
    }
}

impl fmt::Debug for RawPte {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!("RawPte({:#x})", self.0))
    }
}

impl From<RawPte> for u64 {
    fn from(raw_pte: RawPte) -> Self {
        raw_pte.0
    }
}

bitflags! {
    pub struct PresentPageFlags: u64 {
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
        const REGION_HEADER =   1 << 9;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_10 =          1 << 10;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_11 =          1 << 11;
        /// Forbid code execution from the mapped frames.
        ///
        /// Can be only used when the no-execute page protection feature is enabled in the EFER
        /// register.
        const NO_EXECUTE =      1 << 63;
    }
}

// This is a raw present PTE. We can impose more
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RawPresentPte(u64);

impl RawPresentPte {
    // We allocate some space for a "counter" field. We use the unused flag bits in the page table
    // entry for these
    const COUNTER_BITS: u64 = 0x7ff0_0000_0000_0000;
    const COUNTER_SHIFT: u64 = 52;
    pub const MAX_COUNTER_VALUE: u16 = ((Self::COUNTER_BITS >> Self::COUNTER_SHIFT) + 1) as u16;

    pub fn from_frame_and_flags(frame: Frame, flags: PresentPageFlags) -> Self {
        Self::from_frame_flags_and_counter(frame, flags, 0)
    }

    pub fn from_frame_flags_and_counter(
        frame: Frame,
        flags: PresentPageFlags,
        counter: u16,
    ) -> Self {
        assert!(counter < Self::MAX_COUNTER_VALUE);
        Self(
            frame.physical_address() as u64
                | flags.bits()
                | RawPageFlags::PRESENT.bits()
                | ((counter as u64) << Self::COUNTER_SHIFT),
        )
    }

    #[inline]
    pub const fn flags(&self) -> PresentPageFlags {
        PresentPageFlags::from_bits_truncate(self.0)
    }

    #[inline]
    pub fn frame(&self) -> Frame {
        Frame::containing_address(self.0 as usize & 0x000fffff_fffff000)
    }

    #[inline]
    pub const fn counter(&self) -> u16 {
        ((self.0 & Self::COUNTER_BITS) >> Self::COUNTER_SHIFT) as u16
    }

    pub fn is_huge(&self) -> bool {
        self.flags().contains(PresentPageFlags::HUGE_PAGE)
    }
}

impl fmt::Debug for RawPresentPte {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "RawPresentPte(frame: {:?} counter: {:#x} flags: {:?})",
            self.frame(),
            self.counter(),
            self.flags()
        ))
    }
}

impl From<RawPresentPte> for RawPte {
    fn from(rpp: RawPresentPte) -> Self {
        Self(rpp.0)
    }
}

impl TryFrom<RawPte> for RawPresentPte {
    type Error = InvalidPteError;
    fn try_from(rpte: RawPte) -> core::result::Result<Self, Self::Error> {
        if rpte.is_present() {
            Ok(Self(rpte.0))
        } else {
            Err(InvalidPteError(rpte))
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum NotPresentPageType {
    Unused = 0,
    GuardPage = 1,
    RegionHeader = 2,
}

bitflags! {
    pub struct NotPresentPageFlags: u64 {
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_9 =           1 << 9;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_10 =          1 << 10;
        /// Available to the OS, can be used to store additional data, e.g. custom flags.
        const BIT_11 =          1 << 11;
        /// Forbid code execution from the mapped frames.
        ///
        /// Can be only used when the no-execute page protection feature is enabled in the EFER
        /// register.
        const NO_EXECUTE =      1 << 63;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RawNotPresentPte(u64);

impl RawNotPresentPte {
    // We need to keep this 100% consistent with the present entry because if we're using
    // the counter field in both we need to be able to transfer values
    const COUNTER_BITS: u64 = RawPresentPte::COUNTER_BITS;
    const COUNTER_SHIFT: u64 = RawPresentPte::COUNTER_SHIFT;
    pub const MAX_COUNTER_VALUE: u16 = RawPresentPte::MAX_COUNTER_VALUE;

    const TYPE_BITS: u64 = 0x0000_0000_0000_01fe;
    const TYPE_SHIFT: u64 = 1;

    pub fn unused() -> Self {
        Self(0)
    }

    pub fn is_unused(&self) -> bool {
        *self == Self::unused()
    }

    pub fn from_type(page_type: NotPresentPageType) -> Self {
        Self::from_type_flags_frame_and_counter(
            page_type,
            NotPresentPageFlags::empty(),
            Frame::containing_address(0),
            0,
        )
    }

    pub fn from_type_and_counter(page_type: NotPresentPageType, counter: u16) -> Self {
        Self::from_type_flags_frame_and_counter(
            page_type,
            NotPresentPageFlags::empty(),
            Frame::containing_address(0),
            counter,
        )
    }

    pub fn from_type_flags_frame_and_counter(
        page_type: NotPresentPageType,
        flags: NotPresentPageFlags,
        frame: Frame,
        counter: u16,
    ) -> Self {
        assert!(counter < Self::MAX_COUNTER_VALUE);
        Self(
            frame.physical_address() as u64
                | flags.bits()
                | (page_type as u64) << Self::TYPE_SHIFT
                | ((counter as u64) << Self::COUNTER_SHIFT),
        )
    }

    pub fn page_type(&self) -> NotPresentPageType {
        NotPresentPageType::from_u8(((self.0 >> Self::TYPE_SHIFT) & Self::TYPE_BITS) as u8)
            .expect("Invalid PTE type")
    }

    pub fn flags(&self) -> NotPresentPageFlags {
        NotPresentPageFlags::from_bits_truncate(self.0)
    }

    #[inline]
    pub fn frame(&self) -> Frame {
        Frame::containing_address(self.0 as usize & 0x000fffff_fffff000)
    }

    #[inline]
    pub const fn counter(&self) -> u16 {
        ((self.0 & Self::COUNTER_BITS) >> Self::COUNTER_SHIFT) as u16
    }
}

impl fmt::Debug for RawNotPresentPte {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "RawNotPresentPte(frame: {:?} counter: {:#x} type: {:?} flags: {:?}",
            self.frame(),
            self.counter(),
            self.page_type(),
            self.flags()
        ))
    }
}

impl From<RawNotPresentPte> for RawPte {
    fn from(rnp: RawNotPresentPte) -> Self {
        Self(rnp.0)
    }
}

impl TryFrom<RawPte> for RawNotPresentPte {
    type Error = InvalidPteError;
    fn try_from(rpte: RawPte) -> core::result::Result<Self, Self::Error> {
        if rpte.is_present() {
            Err(InvalidPteError(rpte))
        } else {
            Ok(Self(rpte.0))
        }
    }
}

pub struct KernelStackGuardPagePte();

impl KernelStackGuardPagePte {
    pub fn new() -> Self {
        Self()
    }
}

impl From<KernelStackGuardPagePte> for RawNotPresentPte {
    fn from(_: KernelStackGuardPagePte) -> Self {
        RawNotPresentPte::from_type(NotPresentPageType::GuardPage)
    }
}

impl TryFrom<RawNotPresentPte> for KernelStackGuardPagePte {
    type Error = InvalidPteError;
    fn try_from(rpte: RawNotPresentPte) -> core::result::Result<Self, Self::Error> {
        if rpte.page_type() == NotPresentPageType::GuardPage {
            Ok(Self())
        } else {
            Err(InvalidPteError(rpte.into()))
        }
    }
}
