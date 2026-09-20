use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rational {
    pub numer: i128,
    pub denom: i128,
}

impl Rational {
    pub fn new(numer: i128, denom: i128) -> Self {
        assert!(denom > 0, "rational denominator must be positive");
        Self { numer, denom }
    }

    pub fn integer(value: i128) -> Self {
        Self {
            numer: value,
            denom: 1,
        }
    }

    pub fn add(&self, other: &Self) -> Self {
        normalize(
            self.numer * other.denom + other.numer * self.denom,
            self.denom * other.denom,
        )
    }

    pub fn mul_i(&self, value: i128) -> Self {
        normalize(self.numer * value, self.denom)
    }

    pub fn floor_i(&self) -> i128 {
        if self.numer >= 0 {
            self.numer / self.denom
        } else {
            -((-self.numer + self.denom - 1) / self.denom)
        }
    }

    pub fn to_f64(&self) -> f64 {
        self.numer as f64 / self.denom as f64
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.numer * other.denom).cmp(&(other.numer * self.denom))
    }
}

fn normalize(numer: i128, denom: i128) -> Rational {
    assert!(denom > 0, "rational denominator must be positive");
    let divisor = gcd(numer.unsigned_abs(), denom.unsigned_abs()).max(1);
    Rational {
        numer: numer / divisor as i128,
        denom: denom / divisor as i128,
    }
}

fn gcd(left: u128, right: u128) -> u128 {
    if right == 0 {
        left
    } else {
        gcd(right, left % right)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockSegmentSpec {
    pub start_ns: Option<i64>,
    pub end_ns: Option<i64>,
    pub intercept_ns: i64,
    pub slope_num: i64,
    pub slope_den: i64,
    pub uncertainty_ns: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockSegment {
    pub start_ns: Option<i64>,
    pub end_ns: Option<i64>,
    pub intercept: Rational,
    pub slope: Rational,
    pub uncertainty_ns: i64,
}

impl ClockSegment {
    fn contains_start(&self, local_ns: i64) -> bool {
        self.start_ns.is_none_or(|start| local_ns >= start)
    }

    fn contains_end(&self, local_ns: i64) -> bool {
        self.end_ns.is_none_or(|end| local_ns < end)
    }

    fn contains(&self, local_ns: i64) -> bool {
        self.contains_start(local_ns) && self.contains_end(local_ns)
    }

    fn corrected(&self, local_ns: i64) -> Rational {
        self.intercept.add(&self.slope.mul_i(local_ns as i128))
    }
}

impl From<ClockSegmentSpec> for ClockSegment {
    fn from(spec: ClockSegmentSpec) -> Self {
        Self {
            start_ns: spec.start_ns,
            end_ns: spec.end_ns,
            intercept: Rational::integer(spec.intercept_ns as i128),
            slope: Rational::new(spec.slope_num as i128, spec.slope_den as i128),
            uncertainty_ns: spec.uncertainty_ns,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeInterval {
    pub low: i64,
    pub high: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntervalOrder {
    Before,
    After,
    Equal,
    Incomparable,
}

#[derive(Clone, Debug)]
pub struct ClockModel {
    pub device_id: String,
    pub segments: Vec<ClockSegment>,
}

impl ClockModel {
    pub fn new(device_id: impl Into<String>, specs: Vec<ClockSegmentSpec>) -> Result<Self, String> {
        let model = Self {
            device_id: device_id.into(),
            segments: specs.into_iter().map(ClockSegment::from).collect(),
        };
        model.validate()?;
        Ok(model)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.segments.is_empty() {
            return Err(format!("device {} has no clock segment", self.device_id));
        }
        for segment in &self.segments {
            if segment.slope.numer < 0 {
                return Err(format!(
                    "device {} has a negative clock slope",
                    self.device_id
                ));
            }
            if segment.slope.denom <= 0 || segment.uncertainty_ns < 0 {
                return Err(format!(
                    "device {} has an invalid clock segment",
                    self.device_id
                ));
            }
            if let (Some(start), Some(end)) = (segment.start_ns, segment.end_ns) {
                if start >= end {
                    return Err(format!(
                        "device {} has an empty clock segment",
                        self.device_id
                    ));
                }
            }
        }
        if self.segments.first().unwrap().start_ns.is_some()
            || self.segments.last().unwrap().end_ns.is_some()
        {
            return Err(format!(
                "device {} clock does not cover the full local timeline",
                self.device_id
            ));
        }
        for pair in self.segments.windows(2) {
            let left = &pair[0];
            let right = &pair[1];
            match (left.end_ns, right.start_ns) {
                (Some(left_end), Some(right_start)) if left_end == right_start => {
                    let left_value = left.corrected(left_end);
                    let right_value = right.corrected(right_start);
                    if right_value < left_value {
                        return Err(format!(
                            "device {} corrected time moves backward at jump {}",
                            self.device_id, left_end
                        ));
                    }
                }
                _ => {
                    return Err(format!(
                        "device {} has a gap or overlap between clock segments",
                        self.device_id
                    ))
                }
            }
        }
        Ok(())
    }

    pub fn segment_index_for_event(&self, local_ns: i64) -> Option<usize> {
        self.segments
            .iter()
            .position(|segment| segment.contains(local_ns))
    }

    pub fn correct_interval(&self, local_ns: i64) -> Result<TimeInterval, String> {
        let index = self.segment_index_for_event(local_ns).ok_or_else(|| {
            format!(
                "event at {local_ns} is outside device {} clock segments",
                self.device_id
            )
        })?;
        let segment = self.segments[index];
        let corrected = segment.corrected(local_ns).floor_i() as i64;
        let low = (corrected as i128 - segment.uncertainty_ns as i128)
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        let high = (corrected as i128 + segment.uncertainty_ns as i128)
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        Ok(TimeInterval { low, high })
    }
}

pub fn compare_intervals(left: TimeInterval, right: TimeInterval) -> IntervalOrder {
    if left == right && left.low == left.high {
        IntervalOrder::Equal
    } else if left.high < right.low {
        IntervalOrder::Before
    } else if right.high < left.low {
        IntervalOrder::After
    } else {
        IntervalOrder::Incomparable
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnchorObservation {
    pub local_ns: i64,
    pub corrected_ns: i64,
    pub uncertainty_ns: i64,
}

pub fn build_clock_model(
    device_id: impl Into<String>,
    observations: &[AnchorObservation],
    jumps: &[i64],
) -> Result<ClockModel, String> {
    let device_id = device_id.into();
    if observations.is_empty() {
        return Err(format!(
            "device {} requires at least one trusted anchor",
            device_id
        ));
    }
    let mut points = observations.to_vec();
    points.sort_by_key(|point| point.local_ns);
    for pair in points.windows(2) {
        if pair[0].local_ns == pair[1].local_ns {
            return Err(format!(
                "device {device_id} has multiple trusted anchors at local time {}",
                pair[0].local_ns
            ));
        }
    }

    let mut boundaries: Vec<i64> = jumps.to_vec();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut segments = Vec::new();
    for index in 0..=boundaries.len() {
        let start = if index == 0 {
            None
        } else {
            Some(boundaries[index - 1])
        };
        let end = if index < boundaries.len() {
            Some(boundaries[index])
        } else {
            None
        };
        let in_segment: Vec<AnchorObservation> = points
            .iter()
            .copied()
            .filter(|point| {
                start.is_none_or(|boundary| point.local_ns >= boundary)
                    && end.is_none_or(|boundary| point.local_ns < boundary)
            })
            .collect();
        if in_segment.is_empty() {
            return Err(format!(
                "device {} has a clock segment without a trusted anchor",
                device_id
            ));
        }
        let uncertainty_ns = in_segment
            .iter()
            .map(|point| point.uncertainty_ns)
            .max()
            .unwrap_or(0);
        let (intercept, slope) = fit_line(&device_id, &in_segment)?;
        segments.push(ClockSegment {
            start_ns: start,
            end_ns: end,
            intercept,
            slope,
            uncertainty_ns,
        });
    }
    let model = ClockModel {
        device_id,
        segments,
    };
    model.validate()?;
    for point in points {
        let index = model
            .segment_index_for_event(point.local_ns)
            .expect("validated segment");
        let corrected = model.segments[index].corrected(point.local_ns).floor_i() as i64;
        if corrected != point.corrected_ns {
            return Err(format!(
                "device {} anchors are not collinear in one clock segment (local={point_local}, corrected={corrected}, expected={expected})",
                model.device_id,
                point_local = point.local_ns,
                expected = point.corrected_ns
            ));
        }
    }
    Ok(model)
}

fn fit_line(device_id: &str, points: &[AnchorObservation]) -> Result<(Rational, Rational), String> {
    if points.len() == 1 {
        return Ok((
            Rational::integer(points[0].corrected_ns as i128 - points[0].local_ns as i128),
            Rational::integer(1),
        ));
    }
    let first = points[0];
    let second = points[1];
    let dx = second.local_ns as i128 - first.local_ns as i128;
    let dy = second.corrected_ns as i128 - first.corrected_ns as i128;
    if dx <= 0 || dy < 0 {
        return Err(format!(
            "device {} anchors must advance in corrected time",
            device_id
        ));
    }
    let slope = normalize(dy, dx);
    let intercept =
        Rational::integer(first.corrected_ns as i128).add(&slope.mul_i(-first.local_ns as i128));
    for point in points.iter().skip(2) {
        let value = intercept.add(&slope.mul_i(point.local_ns as i128));
        if value != Rational::integer(point.corrected_ns as i128) {
            return Err(format!(
            "device {device_id} anchors are not collinear in one clock segment (local={local}, actual={actual}, expected={expected})",
            device_id = device_id,
            local = point.local_ns,
            actual = value.floor_i(),
            expected = point.corrected_ns
        ));
        }
    }
    Ok((intercept, slope))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jump_endpoint_belongs_to_later_segment() {
        let model = ClockModel::new(
            "d",
            vec![
                ClockSegmentSpec {
                    start_ns: None,
                    end_ns: Some(10),
                    intercept_ns: 100,
                    slope_num: 1,
                    slope_den: 1,
                    uncertainty_ns: 1,
                },
                ClockSegmentSpec {
                    start_ns: Some(10),
                    end_ns: None,
                    intercept_ns: 110,
                    slope_num: 1,
                    slope_den: 1,
                    uncertainty_ns: 2,
                },
            ],
        )
        .unwrap();
        assert_eq!(model.segment_index_for_event(10), Some(1));
        assert_eq!(
            model.correct_interval(10).unwrap(),
            TimeInterval {
                low: 118,
                high: 122
            }
        );
    }

    #[test]
    fn rejects_backward_jump() {
        let error = ClockModel::new(
            "d",
            vec![
                ClockSegmentSpec {
                    start_ns: None,
                    end_ns: Some(10),
                    intercept_ns: 100,
                    slope_num: 1,
                    slope_den: 1,
                    uncertainty_ns: 0,
                },
                ClockSegmentSpec {
                    start_ns: Some(10),
                    end_ns: None,
                    intercept_ns: 99,
                    slope_num: 1,
                    slope_den: 1,
                    uncertainty_ns: 0,
                },
            ],
        )
        .unwrap_err();
        assert!(error.contains("backward"));
    }

    #[test]
    fn overlapping_intervals_are_incomparable() {
        assert_eq!(
            compare_intervals(
                TimeInterval { low: 0, high: 10 },
                TimeInterval { low: 5, high: 15 }
            ),
            IntervalOrder::Incomparable
        );
        assert_eq!(
            compare_intervals(
                TimeInterval { low: 0, high: 10 },
                TimeInterval { low: 11, high: 15 }
            ),
            IntervalOrder::Before
        );
    }
}
