use crate::Changeset;
use anyhow::{Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use quick_xml::Reader;
use std::io::BufReader;
use std::sync::mpsc::SyncSender;

pub const CHANNEL_BATCH_SIZE: usize = 1000;

fn parse_timestamp(s: &str) -> Result<i64> {
    let dt = chrono::DateTime::parse_from_rfc3339(s)
        .with_context(|| format!("Failed to parse timestamp: {}", s))?;
    Ok(dt.timestamp_millis())
}

fn parse_changeset_element(e: &BytesStart) -> Result<Changeset> {
    let mut changeset = Changeset::default();

    for attr in e.attributes() {
        let attr = attr?;
        let key = attr.key.as_ref();
        let value = attr.unescape_value()?;

        match key {
            b"id" => changeset.id = value.as_ref().parse()?,
            b"created_at" => changeset.created_at = Some(parse_timestamp(value.as_ref())?),
            b"closed_at" => changeset.closed_at = Some(parse_timestamp(value.as_ref())?),
            b"open" => changeset.open = value == "true",
            b"user" => changeset.user = Some(value.to_string()),
            b"uid" => changeset.uid = Some(value.as_ref().parse()?),
            b"min_lat" => changeset.min_lat = Some(value.as_ref().parse()?),
            b"min_lon" => changeset.min_lon = Some(value.as_ref().parse()?),
            b"max_lat" => changeset.max_lat = Some(value.as_ref().parse()?),
            b"max_lon" => changeset.max_lon = Some(value.as_ref().parse()?),
            b"num_changes" => changeset.num_changes = value.as_ref().parse()?,
            b"comments_count" => changeset.comments_count = value.as_ref().parse()?,
            _ => {}
        }
    }

    Ok(changeset)
}

fn apply_changeset_tag(e: &BytesStart, changeset: &mut Changeset) -> Result<()> {
    let mut key = None;
    let mut value = None;

    for attr in e.attributes() {
        let attr = attr?;
        match attr.key.as_ref() {
            b"k" => key = Some(attr.unescape_value()?.into_owned()),
            b"v" => value = Some(attr.unescape_value()?.into_owned()),
            _ => {}
        }
    }

    if let (Some(k), Some(v)) = (key, value) {
        if k == "comment" {
            changeset.description = Some(v.clone());
        }
        changeset.tags.insert(k, v);
    }

    Ok(())
}

fn parse_changeset_body<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    changeset: &mut Changeset,
    buf: &mut Vec<u8>,
) -> Result<()> {
    loop {
        match reader.read_event_into(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"tag" {
                    apply_changeset_tag(e, changeset)?;
                }
            }
            Ok(Event::Start(e)) => {
                let owned = e.into_owned();
                let tag_name = owned.name().as_ref().to_vec();
                if owned.name().as_ref() == b"tag" {
                    apply_changeset_tag(&owned, changeset)?;
                }
                // Drop owned before handing mutable access back to reader/BufRead
                drop(owned);
                reader.read_to_end_into(QName(&tag_name), buf)?;
            }
            Ok(Event::End(ref e)) if e.name().as_ref() == b"changeset" => break,
            Ok(Event::Eof) => {
                return Err(anyhow::anyhow!(
                    "Unexpected EOF while parsing changeset body"
                ))
            }
            Ok(_) => {} // Ignore other events
            Err(err) => return Err(err.into()),
        }
        buf.clear();
    }

    Ok(())
}

/// Producer: Parses XML and sends batches of changesets to the channel
pub fn parse_xml_to_channel<R: std::io::Read + Send + 'static>(
    reader: R,
    tx: SyncSender<Result<Vec<Changeset>>>,
    continue_on_error: bool,
) {
    // Use a large buffer (1MB) to avoid splitting XML tags across read boundaries
    let buffered_reader = BufReader::with_capacity(1024 * 1024, reader);
    let mut xml_reader = Reader::from_reader(buffered_reader);
    xml_reader.config_mut().trim_text_start = true;
    xml_reader.config_mut().trim_text_end = true;
    xml_reader.config_mut().check_end_names = false; // More lenient parsing
    xml_reader.config_mut().check_comments = false; // Don't validate comments

    let mut buf = Vec::new();
    let mut temp_buf = Vec::new();
    let mut last_changeset_id = 0i64;
    let mut count = 0;

    let mut batch = Vec::with_capacity(CHANNEL_BATCH_SIZE);

    loop {
        let position = xml_reader.buffer_position();
        match xml_reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) if e.name().as_ref() == b"changeset" => {
                match parse_changeset_element(e) {
                    Ok(changeset) => {
                        last_changeset_id = changeset.id;
                        batch.push(changeset);
                        count += 1;
                    }
                    Err(e) => {
                        if !continue_on_error {
                            let _ = tx.send(Err(e));
                            return;
                        } else {
                            eprintln!("Error parsing changeset element at pos {}: {}", position, e);
                        }
                    }
                }
            }
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"changeset" => {
                match parse_changeset_element(e) {
                    Ok(mut changeset) => {
                        temp_buf.clear();
                        match parse_changeset_body(&mut xml_reader, &mut changeset, &mut temp_buf) {
                            Ok(_) => {
                                last_changeset_id = changeset.id;
                                batch.push(changeset);
                                count += 1;
                            }
                            Err(e) => {
                                if !continue_on_error {
                                    let _ = tx.send(Err(e));
                                    return;
                                } else {
                                    eprintln!(
                                        "Error parsing changeset body for ID {}: {}",
                                        changeset.id, e
                                    );
                                }
                            }
                        }
                        temp_buf.clear();
                    }
                    Err(e) => {
                        if !continue_on_error {
                            let _ = tx.send(Err(e));
                            return;
                        } else {
                            eprintln!("Error parsing changeset element at pos {}: {}", position, e);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                eprintln!("\n=== XML PARSE ERROR ===");
                eprintln!("Position: {}", position);
                eprintln!("Changesets processed: {}", count);
                eprintln!("Last changeset ID: {}", last_changeset_id);
                eprintln!("Error: {}", e);
                eprintln!("\nBuffer content at error (first 500 bytes):");
                eprintln!("{}", String::from_utf8_lossy(&buf[..buf.len().min(500)]));
                eprintln!("\nBuffer content at error (last 500 bytes):");
                let start = if buf.len() > 500 { buf.len() - 500 } else { 0 };
                eprintln!("{}", String::from_utf8_lossy(&buf[start..]));
                eprintln!("======================\n");

                if continue_on_error {
                    eprintln!(
                        "Continuing with {} successfully parsed changesets...",
                        count
                    );
                    break;
                } else {
                    let _ = tx.send(Err(anyhow::anyhow!(
                        "Error parsing XML: {}. Use --continue-on-error to save partial results.",
                        e
                    )));
                    return;
                }
            }
            _ => {} // Ignore other events
        }
        buf.clear();

        if batch.len() >= CHANNEL_BATCH_SIZE {
            if tx.send(Ok(batch)).is_err() {
                // Receiver disconnected
                return;
            }
            batch = Vec::with_capacity(CHANNEL_BATCH_SIZE);
        }
    }

    if !batch.is_empty() {
        let _ = tx.send(Ok(batch));
    }
}
