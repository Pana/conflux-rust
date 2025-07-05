use std::{any::Any, error, fmt, io, sync::mpsc};

/// An error returned from the `Sender::send` or `SyncSender::send` function.
pub enum SendError<T> {
    /// An IO error.
    Io(io::Error),

    /// The receiving half of the channel has disconnected.
    Disconnected(T),
}

/// An error returned from the `SyncSender::try_send` function.
pub enum TrySendError<T> {
    /// An IO error.
    Io(io::Error),

    /// Data could not be sent because it would require the callee to block.
    Full(T),

    /// The receiving half of the channel has disconnected.
    Disconnected(T),
}

/*
 *
 * ===== Error conversions =====
 *
 */

impl<T> From<mpsc::SendError<T>> for SendError<T> {
    fn from(src: mpsc::SendError<T>) -> SendError<T> {
        SendError::Disconnected(src.0)
    }
}

impl<T> From<io::Error> for SendError<T> {
    fn from(src: io::Error) -> SendError<T> { SendError::Io(src) }
}

impl<T> From<mpsc::TrySendError<T>> for TrySendError<T> {
    fn from(src: mpsc::TrySendError<T>) -> TrySendError<T> {
        match src {
            mpsc::TrySendError::Full(v) => TrySendError::Full(v),
            mpsc::TrySendError::Disconnected(v) => {
                TrySendError::Disconnected(v)
            }
        }
    }
}

impl<T> From<mpsc::SendError<T>> for TrySendError<T> {
    fn from(src: mpsc::SendError<T>) -> TrySendError<T> {
        TrySendError::Disconnected(src.0)
    }
}

impl<T> From<io::Error> for TrySendError<T> {
    fn from(src: io::Error) -> TrySendError<T> { TrySendError::Io(src) }
}

/*
 *
 * ===== Implement Error, Debug and Display for Errors =====
 *
 */

impl<T: Any> error::Error for SendError<T> {
    fn description(&self) -> &str {
        match *self {
            SendError::Io(ref io_err) => io_err.description(),
            SendError::Disconnected(..) => "Disconnected",
        }
    }
}

impl<T: Any> error::Error for TrySendError<T> {
    fn description(&self) -> &str {
        match *self {
            TrySendError::Io(ref io_err) => io_err.description(),
            TrySendError::Full(..) => "Full",
            TrySendError::Disconnected(..) => "Disconnected",
        }
    }
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        format_send_error(self, f)
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        format_send_error(self, f)
    }
}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        format_try_send_error(self, f)
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        format_try_send_error(self, f)
    }
}

#[inline]
fn format_send_error<T>(
    e: &SendError<T>, f: &mut fmt::Formatter,
) -> fmt::Result {
    match *e {
        SendError::Io(ref io_err) => write!(f, "{}", io_err),
        SendError::Disconnected(..) => write!(f, "Disconnected"),
    }
}

#[inline]
fn format_try_send_error<T>(
    e: &TrySendError<T>, f: &mut fmt::Formatter,
) -> fmt::Result {
    match *e {
        TrySendError::Io(ref io_err) => write!(f, "{}", io_err),
        TrySendError::Full(..) => write!(f, "Full"),
        TrySendError::Disconnected(..) => write!(f, "Disconnected"),
    }
}
