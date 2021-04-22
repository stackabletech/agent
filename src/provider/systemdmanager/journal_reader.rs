//! This module provides functions for reading from the journal.

use anyhow::{Error, Result};
use kubelet::log::Sender;
use std::str;
use systemd::{journal, journal::JournalRef};

/// Reads journal entries with the given invocation ID and sends the
/// contained messages.
///
/// The options `tail` and `follow` in [`sender`] are taken into account.
/// If `follow` is `true` then messages are sent until the channel of
/// [`sender`] is closed. In this case an
/// [`Err(kubelet::log::SendError::ChannelClosed)`] will be returned.
pub async fn send_messages(sender: &mut Sender, invocation_id: &str) -> Result<()> {
    let mut journal = journal::OpenOptions::default().open()?;
    let journal = journal.match_add("_SYSTEMD_INVOCATION_ID", invocation_id)?;

    if let Some(line_count) = sender.tail() {
        journal.seek_tail()?;
        let skipped = journal.previous_skip(line_count as u64 + 1)?;
        if skipped < line_count + 1 {
            journal.seek_head()?;
        }

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
                String::new()
            }
        } else {
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
