//! Bounded public-GeeseFS part schedule; sizes and offsets are bytes.
use std::{ops::Range, str::FromStr};
#[derive(Clone, Debug)]
pub struct PartSchedule(Vec<(u64, u32)>);
impl FromStr for PartSchedule {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut tiers = Vec::new();
        let mut total = 0u32;
        let mut fields = value.split(',').peekable();
        while let Some(field) = fields.next() {
            let mut pair = field.split(':');
            let size = pair
                .next()
                .unwrap_or("")
                .parse::<u64>()
                .map_err(|_| "invalid part size")?;
            if !(5..=5120).contains(&size) {
                return Err("part size must be 5..=5120 MiB".into());
            }
            let count = pair
                .next()
                .map(|s| s.parse::<u32>())
                .transpose()
                .map_err(|_| "invalid part count")?
                .unwrap_or(0);
            if pair.next().is_some() || total >= 10000 {
                return Err("invalid/too many part tiers".into());
            }
            let count = if count == 0 {
                if fields.peek().is_some() {
                    return Err("only final part count may be omitted or zero".into());
                }
                10000 - total
            } else {
                count
            };
            total = total
                .checked_add(count)
                .filter(|n| *n <= 10000)
                .ok_or("part count exceeds 10000")?;
            tiers.push((size * 1024 * 1024, count));
        }
        if tiers.is_empty() {
            return Err("empty part schedule".into());
        }
        Ok(Self(tiers))
    }
}
impl PartSchedule {
    pub fn max_size(&self) -> u64 {
        self.0
            .iter()
            .map(|(size, count)| size * u64::from(*count))
            .sum()
    }
    pub fn max_parts(&self) -> u32 {
        self.0.iter().map(|(_, count)| count).sum()
    }
    pub fn range(&self, number: u32) -> Option<Range<u64>> {
        let (mut first, mut offset) = (0, 0);
        for &(size, count) in &self.0 {
            if number < first + count {
                let start = offset + u64::from(number - first) * size;
                return Some(start..start + size);
            }
            first += count;
            offset += size * u64::from(count);
        }
        None
    }
    pub fn at_offset(&self, offset: u64) -> Option<(u32, Range<u64>)> {
        let (mut number, mut first) = (0, 0);
        for &(size, count) in &self.0 {
            let end = first + size * u64::from(count);
            if offset < end {
                number += ((offset - first) / size) as u32;
                return self.range(number).map(|range| (number, range));
            }
            number += count;
            first = end;
        }
        None
    }
}
