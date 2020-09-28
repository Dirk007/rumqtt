use std::collections::HashMap;
use std::io;

use crate::volatile::Log;
use crate::{Config, DataReply, DataRequest};
use bytes::Bytes;
use std::sync::Arc;

// TODO change config to Arc
pub(crate) struct DataLog {
    id: usize,
    config: Arc<Config>,
    logs: HashMap<String, Log>,
}

impl DataLog {
    pub fn new(config: Arc<Config>, id: usize) -> DataLog {
        DataLog {
            id,
            config,
            logs: HashMap::new(),
        }
    }

    /// Appends the record to correct commitlog and returns a boolean to indicate
    /// if this topic is new along with the offset of append
    pub fn append(&mut self, topic: &str, record: Bytes) -> io::Result<(bool, (u64, u64))> {
        // Entry instead of if/else?
        if let Some(log) = self.logs.get_mut(topic) {
            let offsets = log.append(record);
            Ok((false, offsets))
        } else {
            let max_segment_size = self.config.max_segment_size;
            let max_segment_count = self.config.max_segment_count;
            let mut log = Log::new(max_segment_size, max_segment_count);
            let offsets = log.append(record);
            self.logs.insert(topic.to_owned(), log);
            Ok((true, offsets))
        }
    }

    fn next_offset(&self, topic: &str) -> Option<(u64, u64)> {
        let log = match self.logs.get(topic) {
            Some(log) => log,
            None => return None,
        };

        Some(log.next_offset())
    }

    pub fn seek_offsets_to_end(&self, topics: &mut Vec<(String, u8, [(u64, u64); 3])>) {
        for (topic, _, offset) in topics.iter_mut() {
            if let Some(last_offset) = self.next_offset(topic) {
                offset[self.id] = last_offset;
            }
        }
    }

    pub fn readv(
        &mut self,
        topic: &str,
        in_segment: u64,
        in_offset: u64,
    ) -> io::Result<Option<(Option<u64>, u64, u64, Vec<Bytes>)>> {
        // Router during data request and notifications will check both
        // native and replica commitlog where this topic doesn't exist
        let log = match self.logs.get_mut(topic) {
            Some(log) => log,
            None => return Ok(None),
        };

        let (jump, segment, offset, data) = log.readv(in_segment, in_offset);

        // For debugging. Will be removed later
        // println!(
        //     "In: segment {} offset {}, Out: segment {} offset {}, Count {}",
        //     in_segment,
        //     in_offset,
        //     segment,
        //     offset,
        //     data.len()
        // );
        Ok(Some((jump, segment, offset, data)))
    }
}