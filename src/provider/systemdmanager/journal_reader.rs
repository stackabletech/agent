//! This module provides functions for reading from the journal.

use anyhow::{Error, Result};
use kubelet::log::Sender;
use std::str;
use systemd::{journal, journal::JournalRef};

/// Reads journal entries with the given invocation ID and sends the
/// contained messages.
///
/// The options `tail` and `follow` in [`sender`] are taken into account.
///
/// If `tail` is set with `Some(line_count)` then only the last
/// `line_count` messages (or less if not enough available) are sent
/// otherwise all available messages are sent.
///
/// If `follow` is `true` then additionally all new messages are sent
/// until the channel of [`sender`] is closed. In this case an
/// [`Err(kubelet::log::SendError::ChannelClosed)`] will be returned.
pub async fn send_messages(sender: &mut Sender, invocation_id: &str) -> Result<()> {
    let mut journal = journal::OpenOptions::default().open()?;
    let journal = journal.match_add("_SYSTEMD_INVOCATION_ID", invocation_id)?;

    if let Some(line_count) = sender.tail() {
        seek_journal_backwards(journal, line_count)?;

        if sender.follow() {
            send_remaining_messages(journal, sender).await?;
        } else {
            send_n_messages(journal, sender, line_count).await?;
        }
    } else {
        send_remaining_messages(journal, sender).await?;
    }

    while sender.follow() {
        journal.wait(None)?;
        send_remaining_messages(journal, sender).await?;
    }

    Ok(())
}

/// Sets the cursor of the journal to the position before the last `count`
/// entries so that the next entry is the first of `count` remaining
/// entries. If the beginning of the journal is reached then the cursor is
/// set to the position before the first entry.
fn seek_journal_backwards(journal: &mut JournalRef, count: usize) -> Result<()> {
    journal.seek_tail()?;

    let entries_to_skip = count + 1;
    let skipped = journal.previous_skip(entries_to_skip as u64)?;
    let beginning_reached = skipped < entries_to_skip;
    if beginning_reached {
        journal.seek_head()?;
    }

    Ok(())
}

/// Sends the given number of messages from the journal.
async fn send_n_messages(
    journal: &mut JournalRef,
    sender: &mut Sender,
    count: usize,
) -> Result<()> {
    let mut sent = 0;
    let mut message_available = true;
    while sent != count && message_available {
        if let Some(message) = next_message(journal)? {
            send_message(sender, &message).await?;
            sent += 1;
        } else {
            message_available = false;
        }
    }
    Ok(())
}

/// Sends the remaining messages from the journal.
async fn send_remaining_messages(journal: &mut JournalRef, sender: &mut Sender) -> Result<()> {
    while let Some(message) = next_message(journal)? {
        send_message(sender, &message).await?;
    }
    Ok(())
}

/// Retrieves the message of the next entry from the journal.
///
/// Returns [`Ok(Some(message))`] if a message could be successfully retrieved
/// and advances the position in the journal. If the journal entry has no
/// message assigned then `message` is an empty string.
/// Returns [`Ok(None)`] if there are no new entries.
/// Returns [`Err(error)`] if the journal could not be read.
fn next_message(journal: &mut JournalRef) -> Result<Option<String>> {
    let maybe_message = if journal.next()? != 0 {
        let message = if let Some(entry) = journal.get_data("MESSAGE")? {
            if let Some(value) = entry.value() {
                String::from_utf8_lossy(value).into()
            } else {
                // The MESSAGE field contains no text, i.e. `MESSAGE=`.
                String::new()
            }
        } else {
            // The journal entry contains no MESSAGE field.
            String::new()
        };
        Some(message)
    } else {
        None
    };
    Ok(maybe_message)
}

/// Sends the given message with a newline character.
async fn send_message(sender: &mut Sender, message: &str) -> Result<()> {
    let mut line = message.to_owned();
    line.push('\n');
    sender.send(line).await.map_err(Error::new)
}
