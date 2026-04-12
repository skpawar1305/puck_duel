#!/bin/bash
sed -i -e '/fn net_msg(&self) -> Option<String>/,/^    fn apply_net/!b' \
       -e '/fn net_msg(&self) -> Option<String>/,/^    }/!d' src-tauri/src/game.rs
