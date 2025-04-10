
use gimli::{
    DebugAbbrev, DebugInfo, DebugLine, DebugStr, Dwarf, EndianSlice, LittleEndian, RunTimeEndian,
    UnitHeader, AttributeValue, DW_AT_location, DW_AT_name, EvaluationResult,
    SectionId,
};
use object::{Object, ObjectSection};
use std::{borrow::Cow, error::Error, fs, path::Path};

