//! Screen geometry and barrier types.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Screen geometry for a machine's display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct ScreenGeometry {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// X offset for multi-monitor setups.
    pub x: i32,
    /// Y offset for multi-monitor setups.
    pub y: i32,
}

impl ScreenGeometry {
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            x: 0,
            y: 0,
        }
    }

    /// Check whether a pixel coordinate is on a given screen edge.
    #[must_use]
    pub fn is_at_edge(&self, px: i32, py: i32, edge: ScreenEdge) -> bool {
        let right = self
            .x
            .saturating_add(i32::try_from(self.width).unwrap_or(i32::MAX))
            .saturating_sub(1);
        let bottom = self
            .y
            .saturating_add(i32::try_from(self.height).unwrap_or(i32::MAX))
            .saturating_sub(1);
        match edge {
            ScreenEdge::Left => px <= self.x,
            ScreenEdge::Right => px >= right,
            ScreenEdge::Top => py <= self.y,
            ScreenEdge::Bottom => py >= bottom,
        }
    }
}

/// Which edge of the screen a barrier is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ScreenEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl ScreenEdge {
    /// Return the opposite edge.
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }
}

/// A barrier defines a region on a screen edge that triggers switching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub struct Barrier {
    pub id: BarrierId,
    /// Which edge this barrier is on.
    pub edge: ScreenEdge,
    /// Start position along the edge (pixels, inclusive).
    pub start: u32,
    /// End position along the edge (pixels, inclusive).
    pub end: u32,
}

/// Unique identifier for a barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub struct BarrierId(pub u32);

/// Position of a remote screen relative to the local screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum Position {
    Left,
    Right,
    Above,
    Below,
}

impl Position {
    /// The screen edge on the local machine that corresponds to this position.
    #[must_use]
    pub fn local_edge(&self) -> ScreenEdge {
        match self {
            Self::Left => ScreenEdge::Left,
            Self::Right => ScreenEdge::Right,
            Self::Above => ScreenEdge::Top,
            Self::Below => ScreenEdge::Bottom,
        }
    }

    /// The screen edge on the remote machine where the cursor enters.
    #[must_use]
    pub fn remote_entry_edge(&self) -> ScreenEdge {
        match self {
            Self::Left => ScreenEdge::Right,
            Self::Right => ScreenEdge::Left,
            Self::Above => ScreenEdge::Bottom,
            Self::Below => ScreenEdge::Top,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_geometry_roundtrip() {
        let geo = ScreenGeometry {
            width: 1920,
            height: 1080,
            x: 0,
            y: 0,
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&geo, config).unwrap();
        let (decoded, _): (ScreenGeometry, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(geo, decoded);
    }

    #[test]
    fn is_at_edge_left() {
        let geo = ScreenGeometry::new(1920, 1080);
        assert!(geo.is_at_edge(0, 500, ScreenEdge::Left));
        assert!(!geo.is_at_edge(1, 500, ScreenEdge::Left));
    }

    #[test]
    fn is_at_edge_right() {
        let geo = ScreenGeometry::new(1920, 1080);
        assert!(geo.is_at_edge(1919, 500, ScreenEdge::Right));
        assert!(!geo.is_at_edge(1918, 500, ScreenEdge::Right));
    }

    #[test]
    fn is_at_edge_top() {
        let geo = ScreenGeometry::new(1920, 1080);
        assert!(geo.is_at_edge(500, 0, ScreenEdge::Top));
        assert!(!geo.is_at_edge(500, 1, ScreenEdge::Top));
    }

    #[test]
    fn is_at_edge_bottom() {
        let geo = ScreenGeometry::new(1920, 1080);
        assert!(geo.is_at_edge(500, 1079, ScreenEdge::Bottom));
        assert!(!geo.is_at_edge(500, 1078, ScreenEdge::Bottom));
    }

    #[test]
    fn barrier_roundtrip() {
        let barrier = Barrier {
            id: BarrierId(1),
            edge: ScreenEdge::Right,
            start: 0,
            end: 1080,
        };
        let config = bincode::config::standard();
        let bytes = bincode::encode_to_vec(&barrier, config).unwrap();
        let (decoded, _): (Barrier, _) = bincode::decode_from_slice(&bytes, config).unwrap();
        assert_eq!(barrier, decoded);
    }

    #[test]
    fn position_edge_mapping() {
        assert_eq!(Position::Left.local_edge(), ScreenEdge::Left);
        assert_eq!(Position::Left.remote_entry_edge(), ScreenEdge::Right);
        assert_eq!(Position::Right.local_edge(), ScreenEdge::Right);
        assert_eq!(Position::Right.remote_entry_edge(), ScreenEdge::Left);
        assert_eq!(Position::Above.local_edge(), ScreenEdge::Top);
        assert_eq!(Position::Above.remote_entry_edge(), ScreenEdge::Bottom);
        assert_eq!(Position::Below.local_edge(), ScreenEdge::Bottom);
        assert_eq!(Position::Below.remote_entry_edge(), ScreenEdge::Top);
    }
}
