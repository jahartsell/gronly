# gronly

A set of low-latency lock-free concurrent containers implemented without memory reclamation schemes.

This crate explores concurrent data structures with restricted operations to avoid needing advanced
memory management such as hazard pointers and epoch based reclamation. Typically this means having
no remove operation in shared contexts, making the containers **grow-only** (hence the name).