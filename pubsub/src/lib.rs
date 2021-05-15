use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::watch::{channel, error::SendError, Receiver, Sender};
use std::error::Error as StdError;
pub use tokio_stream::{wrappers::WatchStream, StreamExt};

pub enum Error {
    SendError(String),
    ChannelNotFound,
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::SendError(e) => write!(f, "{:?}", e),
            Error::ChannelNotFound => {
                write!(f, "channel not found(call create_channel before use it)")
            }
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::SendError(e) => write!(f, "{:?}", e),
            Error::ChannelNotFound => {
                write!(f, "channel not found(call create_channel before use it)")
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::SendError(_) => None,
            Error::ChannelNotFound => None,
        }
    }
}

impl<D: fmt::Debug> From<SendError<D>> for Error {
    fn from(e: SendError<D>) -> Self {
        Error::SendError(format!("{:?}", e))
    }
}

pub type Channel<C, D> = Arc<dashmap::DashMap<C, (Sender<D>, Receiver<D>)>>;

#[derive(Clone)]
pub struct PubSub<C: Eq + Hash, D>
where
    D: 'static + Clone + Unpin + Send + Sync + fmt::Debug,
{
    channels: Channel<C, D>
}

unsafe impl<C: Eq + Hash, D> Sync for PubSub<C, D> where
    D: 'static + Clone + Unpin + Send + Sync + fmt::Debug
{
}

unsafe impl<C: Eq + Hash, D> Send for PubSub<C, D> where
    D: 'static + Clone + Unpin + Send + Sync + fmt::Debug
{
}

impl<C: Eq + Hash, D> PubSub<C, D>
where
    D: 'static + Clone + Unpin + Send + Sync + fmt::Debug,
{
    pub fn new() -> Self {
        PubSub {
            channels: Arc::new(dashmap::DashMap::new()),
        }
    }
    pub fn create_channel(&self, ch: C, init: D) {
        if !self.channels.contains_key(&ch) {
            self.channels.insert(ch, channel(init));
        }
    }
    pub fn remove_channel(&self, ch: &C) {
        self.channels.remove(&ch);
    }
    pub fn publish(&self, ch: &C, data: D) -> Result<(), Error> {
        Ok(self
            .channels
            .get(ch)
            .ok_or(Error::ChannelNotFound)?
            .0
            .send(data)?)
    }
    pub fn subscribe(&self, ch: &C) -> Result<Receiver<D>, Error> {
        Ok(self
            .channels
            .get(ch)
            .ok_or(Error::ChannelNotFound)?
            .1
            .clone())
    }
    pub fn subscribe_stream(&self, ch: &C) -> Result<WatchStream<D>, Error> {
        let rx = self
            .channels
            .get(ch)
            .ok_or(Error::ChannelNotFound)?
            .1
            .clone();
        Ok(WatchStream::new(rx))
    }
}

impl<C: Eq + Hash, D> Default for PubSub<C, D>
where
    D: 'static + Clone + Unpin + Send + Sync + fmt::Debug,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn watch_basic() {
        let watch = PubSub::new();
        watch.create_channel("ch", "data");
        let rx = watch.subscribe(&"ch").unwrap();
        let mut stream = ps.subscribe_stream(&"ch").unwrap();
        watch.publish(&"ch", "data2").unwrap();
        assert_eq!(*rx.borrow(), "data2");
        assert_eq!(stream.next().await, Some("data2"));
        watch.remove_channel(&"ch");
        assert_eq!(stream.next().await, Some("data2"));
        assert_eq!(stream.next().await, None);
    }
}
