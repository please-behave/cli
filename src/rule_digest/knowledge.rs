use crate::limits::MAX_METADATA_BYTES;
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use yaml_rust2::{
    Yaml,
    parser::{Event, Parser, Tag},
    scanner::TScalarStyle,
};

const MAX_DEPTH: usize = 64;

pub(crate) fn parse(text: &str) -> Result<Map<String, Value>> {
    let mut reader = Reader {
        parser: Parser::new(text.chars()),
        remaining: 2 * MAX_METADATA_BYTES as usize,
        anchors: BTreeMap::new(),
    };
    ensure!(
        reader.next()? == Event::StreamStart && reader.next()? == Event::DocumentStart,
        "expected a knowledge document"
    );
    let root = reader.node(0)?;
    ensure!(
        reader.next()? == Event::DocumentEnd && reader.next()? == Event::StreamEnd,
        "expected a single knowledge document"
    );
    match root.value {
        Value::Object(members) => Ok(members),
        _ => bail!("rule knowledge must be an object"),
    }
}

struct Node {
    value: Value,
    height: usize,
}

struct Reader<I> {
    parser: Parser<I>,
    remaining: usize,
    anchors: BTreeMap<usize, (Node, usize)>,
}

impl<I: Iterator<Item = char>> Reader<I> {
    fn next(&mut self) -> Result<Event> {
        Ok(self.parser.next_token()?.0)
    }

    fn consume(&mut self, amount: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(amount)
            .context("expanded rule knowledge exceeds its size limit")?;
        Ok(())
    }

    fn node(&mut self, depth: usize) -> Result<Node> {
        ensure!(depth <= MAX_DEPTH, "rule knowledge is nested too deeply");
        let before = self.remaining;
        let mut height = 0;
        self.consume(1)?;

        let (value, anchor) = match self.next()? {
            Event::Scalar(text, style, anchor, tag) => {
                self.consume(text.len())?;
                (scalar(text, style, tag)?, anchor)
            }
            Event::SequenceStart(anchor, tag) => {
                collection_tag(tag, "seq")?;
                let mut items = Vec::new();
                while self.parser.peek()?.0 != Event::SequenceEnd {
                    let child = self.node(depth + 1)?;
                    height = height.max(child.height + 1);
                    items.push(child.value);
                }
                self.next()?;
                (Value::Array(items), anchor)
            }
            Event::MappingStart(anchor, tag) => {
                collection_tag(tag, "map")?;
                let mut members = Map::new();
                while self.parser.peek()?.0 != Event::MappingEnd {
                    let key = self.node(depth + 1)?;
                    let Value::String(key) = key.value else {
                        bail!("rule knowledge object keys must be strings");
                    };
                    let child = self.node(depth + 1)?;
                    height = height.max(child.height + 1);
                    ensure!(
                        members.insert(key, child.value).is_none(),
                        "duplicate rule knowledge object key"
                    );
                }
                self.next()?;
                (Value::Object(members), anchor)
            }
            Event::Alias(id) => {
                let (target, size) = self.anchors.get(&id).context("unresolved YAML alias")?;
                ensure!(
                    depth + target.height <= MAX_DEPTH,
                    "rule knowledge is nested too deeply"
                );
                // Check the cost before cloning the expanded value.
                self.remaining = self
                    .remaining
                    .checked_sub(*size)
                    .context("expanded rule knowledge exceeds its size limit")?;
                height = target.height;
                (target.value.clone(), 0)
            }
            _ => bail!("expected a knowledge value"),
        };
        if anchor != 0 {
            self.anchors.insert(
                anchor,
                (
                    Node {
                        value: value.clone(),
                        height,
                    },
                    before - self.remaining,
                ),
            );
        }
        Ok(Node { value, height })
    }
}

fn collection_tag(tag: Option<Tag>, expected: &str) -> Result<()> {
    ensure!(
        tag.is_none_or(|tag| standard_tag(&tag, expected)),
        "unsupported rule knowledge collection tag"
    );
    Ok(())
}

fn standard_tag(tag: &Tag, suffix: &str) -> bool {
    tag.handle == "tag:yaml.org,2002:" && tag.suffix == suffix
}

fn scalar(text: String, style: TScalarStyle, tag: Option<Tag>) -> Result<Value> {
    if let Some(tag) = tag {
        if standard_tag(&tag, "str") || tag.handle == "!" && tag.suffix.is_empty() {
            return Ok(Value::String(text));
        }
        ensure!(
            standard_tag(&tag, "bool"),
            "unsupported rule knowledge scalar tag"
        );
        if let Yaml::Boolean(value) = Yaml::from_str(&text) {
            return Ok(Value::Bool(value));
        }
        bail!("invalid YAML boolean");
    }
    if style != TScalarStyle::Plain {
        return Ok(Value::String(text));
    }
    match Yaml::from_str(&text) {
        Yaml::String(text) if !matches!(text.as_str(), "Null" | "NULL") => Ok(Value::String(text)),
        Yaml::Boolean(value) => Ok(Value::Bool(value)),
        _ => bail!("rule knowledge cannot contain numbers or nulls"),
    }
}
