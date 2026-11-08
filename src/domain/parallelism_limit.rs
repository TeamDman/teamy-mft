use arbitrary::Arbitrary;
use facet::Facet;
use std::fmt::Display;
use std::num::NonZeroUsize;
use std::num::ParseIntError;
use std::str::FromStr;

/// A concurrency limit, with zero selecting the operation's automatic policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet, Arbitrary)]
#[facet(opaque, proxy = usize)]
#[repr(u8)]
pub enum ParallelismLimit {
    Automatic,
    Fixed(NonZeroUsize),
}

impl Default for ParallelismLimit {
    fn default() -> Self {
        Self::Fixed(NonZeroUsize::MIN)
    }
}

impl From<usize> for ParallelismLimit {
    fn from(value: usize) -> Self {
        match NonZeroUsize::new(value) {
            Some(value) => Self::Fixed(value),
            None => Self::Automatic,
        }
    }
}

impl From<ParallelismLimit> for usize {
    fn from(value: ParallelismLimit) -> Self {
        match value {
            ParallelismLimit::Automatic => 0,
            ParallelismLimit::Fixed(value) => value.get(),
        }
    }
}

impl From<&ParallelismLimit> for usize {
    fn from(value: &ParallelismLimit) -> Self {
        (*value).into()
    }
}

impl FromStr for ParallelismLimit {
    type Err = ParseIntError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<usize>().map(Self::from)
    }
}

impl Display for ParallelismLimit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        usize::from(self).fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::ParallelismLimit;
    use std::num::NonZeroUsize;

    #[test]
    fn parses_zero_as_automatic_and_positive_values_as_fixed_limits() -> eyre::Result<()> {
        assert_eq!(
            "0".parse::<ParallelismLimit>()?,
            ParallelismLimit::Automatic
        );
        for value in [1, 32, usize::MAX] {
            assert_eq!(
                value.to_string().parse::<ParallelismLimit>()?,
                ParallelismLimit::Fixed(NonZeroUsize::new(value).unwrap())
            );
        }
        for value in ["", "-1", "auto", "1.5"] {
            value
                .parse::<ParallelismLimit>()
                .expect_err("invalid parallelism limits should be rejected");
        }
        format!("{}0", usize::MAX)
            .parse::<ParallelismLimit>()
            .expect_err("parallelism limits exceeding usize should be rejected");
        Ok(())
    }

    #[test]
    fn facet_preserves_the_numeric_wire_representation() -> eyre::Result<()> {
        for value in [0, 1, 32, usize::MAX] {
            let limit = ParallelismLimit::from(value);
            let json = facet_json::to_string(&limit)?;
            assert_eq!(json, value.to_string());
            assert_eq!(facet_json::from_str::<ParallelismLimit>(&json)?, limit);
            assert_eq!(limit.to_string(), json);
        }
        for json in ["-1", "1.5", "\"automatic\""] {
            assert!(
                facet_json::from_str::<ParallelismLimit>(json).is_err(),
                "invalid JSON limit {json} was accepted"
            );
        }
        // Facet defaults null scalars before converting the numeric proxy.
        let numeric_null = facet_json::from_str::<usize>("null")?;
        assert_eq!(
            facet_json::from_str::<ParallelismLimit>("null")?,
            ParallelismLimit::from(numeric_null)
        );
        Ok(())
    }
}
