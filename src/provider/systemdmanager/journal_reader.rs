use kubelet::log::Sender;
use std::str;
use systemd::{journal, journal::JournalRef};

const MAX_LOG_LINE_LENGTH: usize = 16384;

pub async fn send_journal_entries(sender: &mut Sender, invocation_id: &str) -> anyhow::Result<()> {
    let mut journal = journal::OpenOptions::default().open()?;
    let journal = journal.match_add("_SYSTEMD_INVOCATION_ID", invocation_id)?;

    journal.set_data_threshold(MAX_LOG_LINE_LENGTH)?;

    if let Some(line_count) = sender.tail() {
        journal.seek_tail()?;
        journal.previous_skip(line_count as u64 + 1)?;

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

async fn send_n_messages(
    journal: &mut JournalRef,
    sender: &mut Sender,
    count: usize,
) -> anyhow::Result<()> {
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

async fn send_remaining_messages(
    journal: &mut JournalRef,
    sender: &mut Sender,
) -> anyhow::Result<()> {
    while let Some(message) = next_message(journal)? {
        send_message(sender, &message).await?;
    }
    Ok(())
}

fn next_message(journal: &mut JournalRef) -> anyhow::Result<Option<String>> {
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

async fn send_message(sender: &mut Sender, message: &str) -> anyhow::Result<()> {
    let mut line = message.to_owned();
    line.push('\n');
    sender.send(line).await?;
    Ok(())
}
