use core::ops::{Add, AddAssign, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
pub struct PageCount(pub usize);

impl Add for PageCount {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for PageCount {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for PageCount {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for PageCount {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}
