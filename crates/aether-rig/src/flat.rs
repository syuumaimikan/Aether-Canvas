//! Compact serialisation for point lists.
//!
//! Rigs are mostly vertex arrays, and `{"x": 1.0, "y": 2.0}` per point roughly
//! triples the size of a project manifest. Point lists are therefore written
//! as flat `[x0, y0, x1, y1, ...]` arrays. Readers also accept the verbose
//! object form, so hand-written files and older exports still load.

use aether_core::math::Vec2;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serialize a point list as a flat number array.
pub fn serialize<S: Serializer>(points: &[Vec2], serializer: S) -> Result<S::Ok, S::Error> {
    let flat: Vec<f32> = points.iter().flat_map(|p| [p.x, p.y]).collect();
    flat.serialize(serializer)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Points {
    Flat(Vec<f32>),
    Objects(Vec<Vec2>),
}

/// Deserialize a flat number array (or a list of `{x, y}` objects).
pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Vec2>, D::Error> {
    match Points::deserialize(deserializer)? {
        Points::Flat(values) => {
            if values.len() % 2 != 0 {
                return Err(D::Error::custom("a point list needs an even number of values"));
            }
            Ok(values.chunks_exact(2).map(|c| Vec2::new(c[0], c[1])).collect())
        }
        Points::Objects(points) => Ok(points),
    }
}

#[cfg(test)]
mod tests {
    use aether_core::math::Vec2;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Holder {
        #[serde(with = "super")]
        points: Vec<Vec2>,
    }

    #[test]
    fn points_round_trip_as_a_flat_array() {
        let holder = Holder {
            points: vec![Vec2::new(1.0, 2.0), Vec2::new(3.5, -4.0)],
        };
        let text = serde_json::to_string(&holder).expect("serialize");
        assert_eq!(text, r#"{"points":[1.0,2.0,3.5,-4.0]}"#);
        let back: Holder = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back, holder);
    }

    #[test]
    fn the_verbose_form_is_still_accepted() {
        let back: Holder = serde_json::from_str(r#"{"points":[{"x":1.0,"y":2.0}]}"#).expect("deserialize");
        assert_eq!(back.points, vec![Vec2::new(1.0, 2.0)]);
        assert!(serde_json::from_str::<Holder>(r#"{"points":[1.0]}"#).is_err());
    }
}
