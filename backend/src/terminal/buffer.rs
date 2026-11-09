//! 有界输出缓冲与序列号恢复。
//!
//! 输出按块存储，每块带单调递增序列号；超出容量时淘汰最早的块。
//! 客户端重连时携带最后处理的序列号，服务端只回放其后的数据；
//! 若请求位置已被淘汰则返回 [`Replay::Gap`]。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// 单个输出块。
#[derive(Clone, Debug)]
pub struct Chunk {
    pub sequence: u64,
    pub data: Arc<[u8]>,
}

/// 回放结果。
#[derive(Debug)]
pub enum Replay {
    /// 可回放的输出块（可能为空）。
    Chunks(Vec<Chunk>),
    /// 客户端请求的位置已超出缓冲保留范围。
    Gap,
}

#[derive(Clone)]
pub struct OutputBuffer {
    inner: Arc<Mutex<BufferInner>>,
}

struct BufferInner {
    chunks: VecDeque<Chunk>,
    bytes: usize,
    max_bytes: usize,
    next_sequence: u64,
}

impl OutputBuffer {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferInner {
                chunks: VecDeque::new(),
                bytes: 0,
                max_bytes,
                next_sequence: 1,
            })),
        }
    }

    /// 追加输出，返回该块的序列号。
    pub fn append(&self, data: &[u8]) -> u64 {
        if data.is_empty() {
            return self.latest();
        }
        let mut inner = self.inner.lock().unwrap();
        let sequence = inner.next_sequence;
        inner.next_sequence += 1;
        inner.chunks.push_back(Chunk {
            sequence,
            data: Arc::from(data),
        });
        inner.bytes += data.len();
        // 至少保留一块，避免大块输出导致缓冲永远为空
        while inner.bytes > inner.max_bytes && inner.chunks.len() > 1 {
            if let Some(oldest) = inner.chunks.pop_front() {
                inner.bytes -= oldest.data.len();
            }
        }
        sequence
    }

    /// 最新序列号；缓冲为空时为 0。
    pub fn latest(&self) -> u64 {
        self.inner
            .lock()
            .unwrap()
            .chunks
            .back()
            .map(|chunk| chunk.sequence)
            .unwrap_or(0)
    }

    /// 回放 `sequence > last_sequence` 的输出。
    ///
    /// `last_sequence == 0` 表示从头开始，不做空洞检查；
    /// 其余情况若客户端位置超前（如终端已被更换重编号）或最早的保留块
    /// 与客户端位置不连续，则判定为空洞。
    pub fn replay_after(&self, last_sequence: u64) -> Replay {
        let inner = self.inner.lock().unwrap();
        if last_sequence > 0 {
            let latest = inner
                .chunks
                .back()
                .map(|chunk| chunk.sequence)
                .unwrap_or(0);
            if last_sequence > latest
                || inner.chunks.front().is_some_and(|front| {
                    front.sequence > last_sequence.saturating_add(1)
                })
            {
                return Replay::Gap;
            }
        }
        Replay::Chunks(
            inner
                .chunks
                .iter()
                .filter(|chunk| chunk.sequence > last_sequence)
                .cloned()
                .collect(),
        )
    }

    /// 清空缓冲（终端关闭时调用）。
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.chunks.clear();
        inner.bytes = 0;
    }
}
