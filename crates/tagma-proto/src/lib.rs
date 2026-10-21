//! tagma-proto: tagma's [`Tag`](tagma_core::Tag) as a protobuf message,
//! `tagma.v1.Tag` (`proto/tagma/v1/tag.proto`), generated with prost, and
//! the conversions both ways.
//!
//! The conversions are total in both directions: the message carries the
//! three components as decoded content, exactly the fields of
//! [`tagma_core::Tag`]. Neither direction validates: a message can carry a
//! component no tag string spells bare (an empty key, a space), which
//! [`tagma_core::Tag`]'s `Display` then quotes. Validating a tag is the
//! writer's policy, not the wire's.
#![deny(missing_docs)]

/// The directory `tagma/v1/tag.proto` is imported from: the include path a
/// consumer's build script gives protoc when its own `.proto` files import
/// the message.
pub const PROTO_INCLUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../proto");

/// `tagma.v1`.
pub mod v1 {
    #![allow(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/tagma.v1.rs"));
}

impl From<tagma_core::Tag> for v1::Tag {
    fn from(tag: tagma_core::Tag) -> Self {
        v1::Tag {
            namespace: tag.namespace,
            key: tag.key,
            value: tag.value,
        }
    }
}

impl From<v1::Tag> for tagma_core::Tag {
    fn from(tag: v1::Tag) -> Self {
        tagma_core::Tag {
            namespace: tag.namespace,
            key: tag.key,
            value: tag.value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::v1;
    use prost::{Message, Name};

    fn core(s: &str) -> tagma_core::Tag {
        tagma_core::Tag::parse(s).expect("a valid tag")
    }

    #[test]
    fn a_tag_converts_to_the_message_and_back_unchanged() {
        for s in [
            "urgent",
            "range=5",
            "geo:lat=57.64",
            "x=\"\"",
            "\"a:b\":k=\"c d\"",
        ] {
            let tag = core(s);
            let message = v1::Tag::from(tag.clone());
            assert_eq!(tagma_core::Tag::from(message), tag, "{s}");
        }
    }

    #[test]
    fn each_component_lands_in_its_own_field() {
        let message = v1::Tag::from(core("geo:lat=57.64"));
        assert_eq!(message.namespace.as_deref(), Some("geo"));
        assert_eq!(message.key, "lat");
        assert_eq!(message.value.as_deref(), Some("57.64"));
    }

    // proto3 `optional` keeps presence: `x` and `x=""` are different tags,
    // and stay different across the wire.
    #[test]
    fn an_absent_value_and_an_empty_one_survive_encoding_apart() {
        let absent = v1::Tag::from(core("x"));
        let empty = v1::Tag::from(core("x=\"\""));
        let decode = |m: &v1::Tag| v1::Tag::decode(m.encode_to_vec().as_slice()).expect("decode");
        assert_eq!(decode(&absent).value, None);
        assert_eq!(decode(&empty).value, Some(String::new()));
        assert_eq!(decode(&absent).namespace, None);
    }

    #[test]
    fn the_include_directory_holds_the_proto() {
        let proto = std::path::Path::new(super::PROTO_INCLUDE).join("tagma/v1/tag.proto");
        assert!(proto.is_file(), "{}", proto.display());
    }

    #[test]
    fn the_message_carries_its_full_name() {
        assert_eq!(v1::Tag::full_name(), "tagma.v1.Tag");
    }
}
