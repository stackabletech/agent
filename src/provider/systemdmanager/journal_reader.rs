use log::info;
use std::{
    cmp,
    io::{Result, SeekFrom},
    pin::Pin,
    task::{Context, Poll},
};
use systemd::journal;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

#[derive(Clone)]
pub struct JournalReader {
    invocation_id: String,
    cursor: Option<String>,
    buffer: Vec<u8>,
}

impl JournalReader {
    pub fn new(invocation_id: &str) -> JournalReader {
        JournalReader {
            invocation_id: String::from(invocation_id),
            cursor: None,
            buffer: Vec::new(),
        }
    }
}

impl AsyncRead for JournalReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        info!("poll_read");
        info!("buf remaining: [{}]", buf.remaining());

        if !self.buffer.is_empty() && buf.remaining() != 0 {
            let len = cmp::min(self.buffer.len(), buf.remaining());
            let data = self.buffer.drain(0..len).collect::<Vec<u8>>();
            info!("Write buffer with length [{}] to buf", len);
            buf.put_slice(&data);
        }

        // TODO unexpect and unpanic code
        if buf.remaining() != 0 {
            info!("Write journal entries to buf");
            let mut journal = journal::OpenOptions::default()
                .open()
                .expect("Journal could not be opened");
            let journal = journal
                .match_add("_SYSTEMD_INVOCATION_ID", self.invocation_id.clone())
                .expect("Journal could not be applied");

            info!("Set threshold");
            // TODO Define a good default
            journal
                .set_data_threshold(1000)
                .expect("Cannot set threshold");

            info!("Set cursor");
            match &self.cursor {
                None => journal.seek_head().expect("Seek head failed"),
                Some(cursor) => {
                    journal.seek_cursor(cursor).expect("Seek cursor failed");
                    journal.next().expect("Could not get next journal entry.");
                }
            };

            let mut eof = false;
            while buf.remaining() != 0 && !eof {
                info!("Get next journal entry");
                if journal.next().expect("Could not get next journal entry.") != 0 {
                    info!("Get journal data");
                    match journal
                        .get_data("MESSAGE")
                        .expect("Data could not be retrieved")
                    {
                        Some(message) => {
                            info!("Write message [{:?}] to buf", message);
                            if let Some(value) = message.value() {
                                let mut data = value.to_vec();
                                // TODO explain number
                                data.push(10);
                                let len = cmp::min(data.len(), buf.remaining());
                                let data_to_buffer = data.split_off(len);
                                info!("Write data with length [{}] to buf", data.len());
                                info!(
                                    "Write data with length [{}] to buffer",
                                    data_to_buffer.len()
                                );
                                self.buffer = data_to_buffer;
                                buf.put_slice(&data);
                            }
                        }
                        None => {
                            info!("no message content; eof reached");
                            eof = true;
                        }
                    }
                } else {
                    info!("eof reached");
                    eof = true;
                }
            }

            info!("Update cursor");
            self.cursor = Some(journal.cursor().expect("Cannot retrieve cursor"));
            info!("New cursor: [{:?}]", self.cursor);
        }

        info!("Signal poll ready");
        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for JournalReader {
    fn start_seek(self: Pin<&mut Self>, position: SeekFrom) -> Result<()> {
        info!("Seek to {:?}", position);
        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<u64>> {
        info!("Poll complete");
        Poll::Ready(Ok(0))
    }
}
