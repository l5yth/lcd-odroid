// Copyright 2026 l5y
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Runtime drivers for the four daemon modes.
//!
//! Each submodule owns one mode's network I/O and render loop. The pure
//! formatting and parsing helpers they depend on live in the `lcd_odroid`
//! library; everything in here is binary-side and not part of the lib's
//! 100 %-coverage envelope.

pub mod bitcoin;
pub mod consensus;
pub mod execution;
pub mod hostname;
