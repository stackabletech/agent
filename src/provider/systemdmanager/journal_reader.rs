use log::info;
use std::{
    io::{Result, SeekFrom},
    pin::Pin,
    task::{Context, Poll},
};
use systemd::journal;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

#[derive(Clone)]
pub struct JournalReader {
    pub end: bool,
}

impl AsyncRead for JournalReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        if !self.end {
            self.end = true;
            info!("Put Hallo into the buffer");
            let mut journal = journal::OpenOptions::default()
                .open()
                .expect("Journal could not be opened");
            if let Ok(journal) = journal.match_add(
                "_SYSTEMD_USER_UNIT",
                "default-agent-integration-test-test-service.service",
            ) {
                if let Ok(Some(entry)) = journal.next_entry() {
                    buf.put_slice(format!("{:?}", entry).as_bytes());
                }
            }
        }
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
