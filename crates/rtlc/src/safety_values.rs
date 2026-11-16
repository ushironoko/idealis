use crate::ir::{Expression, Scalar};
use crate::safety_expressions::MembershipSet;
#[cfg(not(target_arch = "wasm32"))]
use crate::safety_work::WorkBudget;
use crate::{CoreError, Value};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq, Hash)]
enum Cell {
    String(String),
    Set(Vec<Scalar>),
    Function(Vec<(Scalar, Scalar)>),
}

struct ValueHasher(u64);
impl Default for ValueHasher {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}
impl Hasher for ValueHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3) ^ value;
    }
    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }
    fn write_i32(&mut self, value: i32) {
        self.write_u32(value as u32);
    }
    fn write_isize(&mut self, value: isize) {
        self.write_u64(value as u64);
    }
}

fn get_position(values: &[(Scalar, Scalar)], key: Scalar) -> Option<usize> {
    if let Scalar::Int(integer) = key
        && integer > 0
    {
        let position = integer as usize - 1;
        if values.get(position).is_some_and(|entry| entry.0 == key) {
            return Some(position);
        }
    }
    values.iter().position(|entry| entry.0 == key)
}

type RegisteredMembership = HashMap<usize, Option<Option<Arc<MembershipSet>>>>;

type MembershipCache = HashMap<(u64, Scalar), Vec<(Arc<MembershipSet>, bool)>>;

pub(crate) struct Arena<'a> {
    base: Option<&'a Arena<'a>>,
    offset: usize,
    #[cfg(not(target_arch = "wasm32"))]
    budget: Option<(&'a WorkBudget, usize)>,
    cells: Vec<Cell>,
    depths: Vec<u8>,
    index: HashMap<u64, u32, BuildHasherDefault<ValueHasher>>,
    links: Vec<Option<u32>>,
    data: HashMap<Value, Scalar>,
    types: HashMap<Expression, Option<Arc<MembershipSet>>>,
    registered: RegisteredMembership,
    membership: MembershipCache,
    pub work: u32,
    maximum: u32,
    pub operators: &'a [crate::ir::OperatorDefinition],
}

impl<'scope> Arena<'scope> {
    pub fn new(maximum: u32) -> Self {
        Self {
            base: None,
            offset: 0,
            #[cfg(not(target_arch = "wasm32"))]
            budget: None,
            cells: vec![],
            depths: vec![],
            index: HashMap::default(),
            links: vec![],
            data: HashMap::new(),
            types: HashMap::new(),
            registered: HashMap::new(),
            membership: HashMap::new(),
            work: 0,
            maximum,
            operators: &[],
        }
    }
    pub fn tick(&mut self) -> Result<(), CoreError> {
        self.consume(1)
    }
    pub fn consume(&mut self, count: usize) -> Result<(), CoreError> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((budget, worker)) = self.budget {
            let used = budget.consume(worker, count)?;
            self.work += used;
            return if used as usize == count {
                Ok(())
            } else {
                Err(work_exhausted())
            };
        }
        if count > (self.maximum - self.work) as usize {
            self.work = self.maximum;
            return Err(work_exhausted());
        }
        self.work += count as u32;
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn fork<'a>(&'a self, budget: &'a WorkBudget, worker: usize) -> Arena<'a> {
        Arena {
            base: Some(self),
            offset: self.offset + self.cells.len(),
            budget: Some((budget, worker)),
            operators: self.operators,
            ..Arena::new(self.maximum)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn into_delta(self) -> ValueDelta {
        ValueDelta {
            cells: self.cells,
            data: self.data,
            types: self.types,
            registered: self.registered,
            membership: self.membership,
            offset: self.offset,
            work: self.work,
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    pub fn merge(&mut self, delta: ValueDelta) -> Result<ValueRemap, CoreError> {
        let mut mapping = ValueRemap {
            offset: delta.offset,
            values: Vec::with_capacity(delta.cells.len()),
        };
        for cell in delta.cells {
            let cell = match cell {
                Cell::String(value) => Cell::String(value),
                Cell::Set(values) => {
                    Cell::Set(values.into_iter().map(|value| mapping.get(value)).collect())
                }
                Cell::Function(values) => Cell::Function(
                    values
                        .into_iter()
                        .map(|(key, value)| (mapping.get(key), mapping.get(value)))
                        .collect(),
                ),
            };
            mapping.values.push(self.insert(cell, false)?);
        }
        for (value, scalar) in delta.data {
            self.data
                .entry(value)
                .or_insert_with(|| mapping.get(scalar));
        }
        for (address, prepared) in delta.registered {
            let target = self.registered.entry(address).or_insert(None);
            if target.is_none() {
                *target =
                    prepared.map(|prepared| prepared.map(|prepared| prepared.remap(&mapping)));
            }
        }
        for (expression, prepared) in delta.types {
            self.types
                .entry(expression)
                .or_insert_with(|| prepared.map(|prepared| prepared.remap(&mapping)));
        }
        for ((_, value), results) in delta.membership {
            for (prepared, result) in results {
                self.cache_membership_result(prepared.remap(&mapping), mapping.get(value), result);
            }
        }
        Ok(mapping)
    }
    pub fn register_memberships(&mut self, expression: &'scope Expression) {
        crate::safety_expressions::register_memberships(expression, &mut |expression| {
            self.registered
                .entry(std::ptr::from_ref(expression) as usize)
                .or_insert(None);
        });
    }
    fn has_registered(&self, address: usize) -> bool {
        self.registered.contains_key(&address)
            || self.base.is_some_and(|base| base.has_registered(address))
    }
    pub fn prepared_membership(
        &self,
        expression: &Expression,
    ) -> Option<Option<Arc<MembershipSet>>> {
        if let Some(prepared) = self
            .registered
            .get(&(std::ptr::from_ref(expression) as usize))
        {
            return prepared.clone();
        }
        self.base
            .and_then(|base| base.prepared_membership(expression))
            .or_else(|| self.types.get(expression).cloned())
    }
    pub fn cache_membership_type(
        &mut self,
        expression: &Expression,
        prepared: Option<Arc<MembershipSet>>,
    ) {
        let address = std::ptr::from_ref(expression) as usize;
        if self.has_registered(address) {
            self.registered.insert(address, Some(prepared));
        } else {
            self.types.insert(expression.clone(), prepared);
        }
    }
    pub fn membership_result(&self, prepared: &Arc<MembershipSet>, value: Scalar) -> Option<bool> {
        self.membership
            .get(&(prepared.fingerprint(), value))
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|(stored, _)| stored == prepared)
                    .map(|(_, result)| *result)
            })
            .or_else(|| {
                self.base
                    .and_then(|base| base.membership_result(prepared, value))
            })
    }
    pub fn cache_membership_result(
        &mut self,
        prepared: Arc<MembershipSet>,
        value: Scalar,
        result: bool,
    ) {
        let entries = self
            .membership
            .entry((prepared.fingerprint(), value))
            .or_default();
        if !entries.iter().any(|(stored, _)| *stored == prepared) {
            entries.push((prepared, result));
        }
    }
    pub fn has_domain(&self, function: Scalar, domain: Scalar) -> Result<bool, CoreError> {
        let Cell::Function(entries) = self.cell(function)? else {
            return Err(CoreError::protocol("Expected a function"));
        };
        let Ok(Cell::Set(keys)) = self.cell(domain) else {
            return Err(CoreError::protocol("Incomparable TLA values"));
        };
        if entries.len() != keys.len() {
            return Ok(false);
        }
        for ((actual, _), expected) in entries.iter().zip(keys) {
            if !self.equal(*actual, *expected)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn text(&self, value: Scalar) -> Option<&str> {
        if let Scalar::Ref(index) = value
            && let Ok(Cell::String(text)) = self.cell(Scalar::Ref(index))
        {
            return Some(text);
        }
        None
    }
    pub fn string_utf16(&mut self, units: &[u16]) -> Result<Scalar, CoreError> {
        self.consume(units.len())?;
        let text = String::from_utf16(units).map_err(|_| {
            CoreError::protocol("UTF-16 slice cannot be represented as Unicode scalar text")
        })?;
        self.string(text)
    }
    pub fn sequence_values(&mut self, value: Scalar) -> Result<Vec<Scalar>, CoreError> {
        if let Some(text) = self.text(value) {
            let units = text.encode_utf16().collect::<Vec<_>>();
            return units
                .into_iter()
                .map(|unit| self.string_utf16(&[unit]))
                .collect();
        }
        self.sequence(value)
    }
    fn cell(&self, value: Scalar) -> Result<&Cell, CoreError> {
        match value {
            Scalar::Ref(index) if (index as usize) < self.offset => self
                .base
                .ok_or_else(|| CoreError::protocol("Missing base value store"))?
                .cell(value),
            Scalar::Ref(index) => self
                .cells
                .get(index as usize - self.offset)
                .ok_or_else(|| CoreError::protocol("Invalid interned value")),
            _ => Err(CoreError::protocol("Expected a collection or string")),
        }
    }
    fn intern(&mut self, cell: Cell) -> Result<Scalar, CoreError> {
        self.insert(cell, true)
    }
    fn find(&self, hash: u64, cell: &Cell) -> Option<u32> {
        let mut candidate = self.index.get(&hash).copied();
        while let Some(index) = candidate {
            let local = index as usize - self.offset;
            if self.cells[local] == *cell {
                return Some(index);
            }
            candidate = self.links[local];
        }
        self.base.and_then(|base| base.find(hash, cell))
    }
    fn depth(&self, index: u32) -> u8 {
        if (index as usize) < self.offset {
            self.base.unwrap().depth(index)
        } else {
            self.depths[index as usize - self.offset]
        }
    }
    fn insert(&mut self, cell: Cell, charge: bool) -> Result<Scalar, CoreError> {
        let mut hasher = ValueHasher::default();
        cell.hash(&mut hasher);
        let hash = hasher.finish();
        if let Some(index) = self.find(hash, &cell) {
            return Ok(Scalar::Ref(index));
        }
        let depth = |value: &Scalar| match value {
            Scalar::Ref(index) => self.depth(*index),
            _ => 0,
        };
        let depth = match &cell {
            Cell::String(_) => 0,
            Cell::Set(values) => 1 + values.iter().map(depth).max().unwrap_or(0),
            Cell::Function(values) => {
                1 + values
                    .iter()
                    .flat_map(|(key, value)| [key, value])
                    .map(depth)
                    .max()
                    .unwrap_or(0)
            }
        };
        if depth > 64 {
            return Err(CoreError::protocol("TLA value nesting exceeds 64"));
        }
        if charge {
            self.tick()?;
        }
        let index = u32::try_from(self.offset + self.cells.len())
            .map_err(|_| CoreError::protocol("Value store exhausted"))?;
        self.links.push(self.index.insert(hash, index));
        self.cells.push(cell);
        self.depths.push(depth);
        Ok(Scalar::Ref(index))
    }
    pub fn string(&mut self, value: String) -> Result<Scalar, CoreError> {
        self.intern(Cell::String(value))
    }
    pub fn data(&mut self, value: &Value) -> Result<Scalar, CoreError> {
        if let Some(result) = self
            .data
            .get(value)
            .or_else(|| self.base.and_then(|base| base.data.get(value)))
        {
            return Ok(*result);
        }
        let result = match value {
            Value::Bool(_) | Value::Int(_) => Scalar::parse(value)?,
            Value::String(value) => self.string(value.clone())?,
            Value::List(values) => {
                let values = values
                    .iter()
                    .map(|value| self.data(value))
                    .collect::<Result<Vec<_>, _>>()?;
                self.tuple(values)?
            }
            Value::Set(values) => {
                let values = values
                    .iter()
                    .map(|value| self.data(value))
                    .collect::<Result<Vec<_>, _>>()?;
                self.set(values)?
            }
            Value::Record(values) => {
                let mut entries = vec![];
                for (name, value) in values {
                    let name = self.string(name.clone())?;
                    let value = self.data(value)?;
                    entries.push((name, value));
                }
                self.function(entries)?
            }
            Value::Map(values) => {
                let mut entries = vec![];
                for (key, value) in values {
                    let key = self.data(key)?;
                    let value = self.data(value)?;
                    entries.push((key, value));
                }
                self.function(entries)?
            }
            _ => return Err(CoreError::protocol("Unsupported TLA data value")),
        };
        self.data.insert(value.clone(), result);
        Ok(result)
    }
    fn rank(&self, value: Scalar) -> u8 {
        match value {
            Scalar::Bool(_) => 0,
            Scalar::Int(_) => 1,
            Scalar::Ref(_) => match self.cell(value) {
                Ok(Cell::String(_)) => 2,
                Ok(Cell::Set(_)) => 3,
                _ => 4,
            },
        }
    }
    pub fn total(&self, left: Scalar, right: Scalar) -> Ordering {
        let rank = self.rank(left).cmp(&self.rank(right));
        if rank != Ordering::Equal {
            return rank;
        }
        match (left, right) {
            (Scalar::Bool(left), Scalar::Bool(right)) => left.cmp(&right),
            (Scalar::Int(left), Scalar::Int(right)) => left.cmp(&right),
            _ => match (self.cell(left), self.cell(right)) {
                (Ok(Cell::String(left)), Ok(Cell::String(right))) => {
                    left.encode_utf16().cmp(right.encode_utf16())
                }
                (Ok(Cell::Set(left)), Ok(Cell::Set(right))) => left
                    .len()
                    .cmp(&right.len())
                    .then_with(|| self.compare_lists(left, right)),
                (Ok(Cell::Function(left)), Ok(Cell::Function(right))) => {
                    let size = left.len().cmp(&right.len());
                    if size != Ordering::Equal {
                        return size;
                    }
                    for ((left_key, _), (right_key, _)) in left.iter().zip(right) {
                        let comparison = self.total(*left_key, *right_key);
                        if comparison != Ordering::Equal {
                            return comparison;
                        }
                    }
                    for ((_, left_value), (_, right_value)) in left.iter().zip(right) {
                        let comparison = self.total(*left_value, *right_value);
                        if comparison != Ordering::Equal {
                            return comparison;
                        }
                    }
                    Ordering::Equal
                }
                _ => Ordering::Equal,
            },
        }
    }
    fn compare_lists(&self, left: &[Scalar], right: &[Scalar]) -> Ordering {
        for (left, right) in left.iter().zip(right) {
            let result = self.total(*left, *right);
            if result != Ordering::Equal {
                return result;
            }
        }
        left.len().cmp(&right.len())
    }
    pub fn comparable(&self, left: Scalar, right: Scalar) -> Result<(), CoreError> {
        if self.rank(left) != self.rank(right) {
            return Err(CoreError::protocol("Incomparable TLA values"));
        }
        if matches!(left, Scalar::Int(_) | Scalar::Bool(_)) {
            return Ok(());
        }
        let pairs: Vec<(Scalar, Scalar)> = match (self.cell(left), self.cell(right)) {
            (Ok(Cell::Set(left)), Ok(Cell::Set(right))) if left.len() == right.len() => {
                left.iter().copied().zip(right.iter().copied()).collect()
            }
            (Ok(Cell::Function(left)), Ok(Cell::Function(right))) if left.len() == right.len() => {
                left.iter()
                    .zip(right)
                    .map(|(left, right)| (left.0, right.0))
                    .chain(
                        left.iter()
                            .zip(right)
                            .map(|(left, right)| (left.1, right.1)),
                    )
                    .collect()
            }
            _ => vec![],
        };
        for (left, right) in pairs {
            self.comparable(left, right)?;
            if self.total(left, right) != Ordering::Equal {
                break;
            }
        }
        Ok(())
    }
    pub fn equal(&self, left: Scalar, right: Scalar) -> Result<bool, CoreError> {
        if left == right {
            return Ok(true);
        }
        if self.rank(left) != self.rank(right) {
            return Err(CoreError::protocol("Incomparable TLA values"));
        }
        if matches!(left, Scalar::Int(_) | Scalar::Bool(_)) {
            return Ok(false);
        }
        match (self.cell(left), self.cell(right)) {
            (Ok(Cell::Function(left)), Ok(Cell::Function(right))) => {
                if left.len() != right.len() {
                    return Ok(false);
                }
                for (left, right) in left.iter().zip(right) {
                    if !self.equal(left.0, right.0)? {
                        return Ok(false);
                    }
                }
                for (left, right) in left.iter().zip(right) {
                    if !self.equal(left.1, right.1)? {
                        return Ok(false);
                    }
                }
            }
            (Ok(Cell::Set(left)), Ok(Cell::Set(right))) => {
                if left.len() != right.len() {
                    return Ok(false);
                }
                for (left, right) in left.iter().zip(right) {
                    if !self.equal(*left, *right)? {
                        return Ok(false);
                    }
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    pub fn set(&mut self, mut values: Vec<Scalar>) -> Result<Scalar, CoreError> {
        values = self.ordered(values)?;
        self.intern(Cell::Set(values))
    }
    pub fn ordered(&self, mut values: Vec<Scalar>) -> Result<Vec<Scalar>, CoreError> {
        values.sort_by(|left, right| self.total(*left, *right));
        for pair in values.windows(2) {
            self.comparable(pair[0], pair[1])?;
        }
        values.dedup();
        Ok(values)
    }
    pub fn tuple(&mut self, values: Vec<Scalar>) -> Result<Scalar, CoreError> {
        self.function(
            values
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    Ok((
                        Scalar::Int(
                            i32::try_from(index + 1)
                                .map_err(|_| CoreError::protocol("Sequence too long"))?,
                        ),
                        value,
                    ))
                })
                .collect::<Result<_, CoreError>>()?,
        )
    }
    pub fn function(&mut self, mut values: Vec<(Scalar, Scalar)>) -> Result<Scalar, CoreError> {
        values.sort_by(|left, right| self.total(left.0, right.0));
        for pair in values.windows(2) {
            self.comparable(pair[0].0, pair[1].0)?;
            if pair[0].0 == pair[1].0 {
                return Err(CoreError::protocol("Duplicate function key"));
            }
        }
        self.intern(Cell::Function(values))
    }
    pub fn members(&self, value: Scalar) -> Result<Vec<Scalar>, CoreError> {
        match self.cell(value)? {
            Cell::Set(values) => Ok(values.clone()),
            _ => Err(CoreError::protocol("Expected a finite set")),
        }
    }
    pub fn contains(&self, value: Scalar, set: Scalar) -> Result<bool, CoreError> {
        let Cell::Set(values) = self.cell(set)? else {
            return Err(CoreError::protocol("Membership requires a finite set"));
        };
        for candidate in values {
            if self.equal(value, *candidate)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn select(&self, function: Scalar, key: Scalar) -> Result<Option<Scalar>, CoreError> {
        let Cell::Function(values) = self.cell(function)? else {
            return Err(CoreError::protocol("Expected a function"));
        };
        Ok(get_position(values, key).map(|position| values[position].1))
    }
    pub fn at(&self, function: Scalar, key: Scalar) -> Result<Scalar, CoreError> {
        self.select(function, key)?
            .ok_or_else(|| CoreError::protocol("Function argument is outside its domain"))
    }
    pub fn domain(&mut self, function: Scalar) -> Result<Scalar, CoreError> {
        let Cell::Function(values) = self.cell(function)? else {
            return Err(CoreError::protocol("DOMAIN requires a function"));
        };
        self.set(values.iter().map(|entry| entry.0).collect())
    }
    pub fn entries(&self, function: Scalar) -> Result<Vec<(Scalar, Scalar)>, CoreError> {
        match self.cell(function)? {
            Cell::Function(values) => Ok(values.clone()),
            _ => Err(CoreError::protocol("Expected a function")),
        }
    }
    pub fn check_tuple(&self, value: Scalar, size: Option<usize>) -> Result<(), CoreError> {
        if let Some(size) = size
            && self.sequence(value)?.len() != size
        {
            return Err(CoreError::protocol(
                "Bound tuple pattern has the wrong length",
            ));
        }
        Ok(())
    }
    pub fn sequence(&self, value: Scalar) -> Result<Vec<Scalar>, CoreError> {
        let values = self.entries(value)?;
        if !values
            .iter()
            .enumerate()
            .all(|(index, entry)| entry.0 == Scalar::Int((index + 1) as i32))
        {
            return Err(CoreError::protocol("Expected a sequence"));
        }
        Ok(values.into_iter().map(|entry| entry.1).collect())
    }
    pub fn replace(
        &mut self,
        function: Scalar,
        path: &[Scalar],
        value: Scalar,
    ) -> Result<Scalar, CoreError> {
        let Some((first, rest)) = path.split_first() else {
            return Ok(value);
        };
        let Cell::Function(entries) = self.cell(function)? else {
            return Err(CoreError::protocol("Expected a function"));
        };
        let Some(position) = get_position(entries, *first) else {
            return Ok(function);
        };
        let previous = entries[position].1;
        let mut entries = entries.clone();
        let updated = self.replace(previous, rest, value)?;
        if updated == previous {
            return Ok(function);
        }
        entries[position].1 = updated;
        self.intern(Cell::Function(entries))
    }
    pub fn path(&self, mut function: Scalar, path: &[Scalar]) -> Result<Option<Scalar>, CoreError> {
        for key in path {
            let Some(value) = self.select(function, *key)? else {
                return Ok(None);
            };
            function = value;
        }
        Ok(Some(function))
    }
    pub fn portable(&self, value: Scalar) -> Result<Value, CoreError> {
        Ok(match value {
            Scalar::Int(value) => Value::Int(value.to_string()),
            Scalar::Bool(value) => Value::Bool(value),
            _ => match self.cell(value)? {
                Cell::String(value) => Value::String(value.clone()),
                Cell::Set(values) => Value::Set(
                    values
                        .iter()
                        .map(|value| self.portable(*value))
                        .collect::<Result<_, _>>()?,
                )
                .normalize()?,
                Cell::Function(values) => {
                    if values
                        .iter()
                        .enumerate()
                        .all(|(index, entry)| entry.0 == Scalar::Int((index + 1) as i32))
                    {
                        Value::List(
                            values
                                .iter()
                                .map(|entry| self.portable(entry.1))
                                .collect::<Result<_, _>>()?,
                        )
                    } else if values
                        .iter()
                        .all(|entry| matches!(self.cell(entry.0), Ok(Cell::String(_))))
                    {
                        Value::Record(
                            values
                                .iter()
                                .map(|entry| {
                                    let Cell::String(name) = self.cell(entry.0)? else {
                                        unreachable!()
                                    };
                                    Ok((name.clone(), self.portable(entry.1)?))
                                })
                                .collect::<Result<_, CoreError>>()?,
                        )
                        .normalize()?
                    } else {
                        Value::Map(
                            values
                                .iter()
                                .map(|entry| Ok((self.portable(entry.0)?, self.portable(entry.1)?)))
                                .collect::<Result<_, CoreError>>()?,
                        )
                        .normalize()?
                    }
                }
            },
        })
    }
}

fn work_exhausted() -> CoreError {
    CoreError {
        code: "budget",
        message: "Safety work budget exhausted".into(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct ValueDelta {
    cells: Vec<Cell>,
    data: HashMap<Value, Scalar>,
    types: HashMap<Expression, Option<Arc<MembershipSet>>>,
    registered: RegisteredMembership,
    membership: MembershipCache,
    offset: usize,
    pub work: u32,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct ValueRemap {
    offset: usize,
    values: Vec<Scalar>,
}

#[cfg(not(target_arch = "wasm32"))]
impl ValueRemap {
    pub fn get(&self, value: Scalar) -> Scalar {
        match value {
            Scalar::Ref(index) if index as usize >= self.offset => {
                self.values[index as usize - self.offset]
            }
            _ => value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Arena, Cell, ValueHasher};
    use crate::ir::Scalar;
    use std::hash::{Hash, Hasher};

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn worker_values_share_the_base_and_remap_conflicting_local_references() {
        use crate::safety_work::WorkBudget;
        let mut base = Arena::new(1000);
        let original = base.tuple(vec![Scalar::Int(0)]).unwrap();
        let budget = WorkBudget::new(900, 2);
        let mut first = base.fork(&budget, 0);
        let mut second = base.fork(&budget, 1);
        assert_eq!(first.tuple(vec![Scalar::Int(0)]).unwrap(), original);
        let left = first.set(vec![Scalar::Int(1)]).unwrap();
        let right = second.set(vec![Scalar::Int(2)]).unwrap();
        assert_eq!(left, right);
        let nested = second.tuple(vec![right, original]).unwrap();
        let first = first.into_delta();
        let second = second.into_delta();
        let first = base.merge(first).unwrap();
        let second = base.merge(second).unwrap();
        assert_ne!(first.get(left), second.get(right));
        assert_eq!(
            base.portable(first.get(left)).unwrap(),
            crate::Value::Set(vec![crate::Value::Int("1".into())])
        );
        assert_eq!(
            base.portable(second.get(right)).unwrap(),
            crate::Value::Set(vec![crate::Value::Int("2".into())])
        );
        let members = base.sequence(second.get(nested)).unwrap();
        assert_eq!(members, vec![second.get(right), original]);
        let canonical = base.set(vec![Scalar::Int(2)]).unwrap();
        assert_eq!(canonical, second.get(right));
    }

    #[test]
    fn colliding_function_hashes_keep_distinct_values() {
        let first = vec![
            (Scalar::Int(0), Scalar::Int(-1_370_170_012)),
            (Scalar::Int(95_947_124), Scalar::Int(0)),
        ];
        let second = vec![
            (Scalar::Int(0), Scalar::Int(803_057_848)),
            (Scalar::Int(1_091_714_232), Scalar::Int(-1_935_938_920)),
        ];
        let hash = |entries: Vec<(Scalar, Scalar)>| {
            let mut hasher = ValueHasher::default();
            Cell::Function(entries).hash(&mut hasher);
            hasher.finish()
        };
        assert_eq!(hash(first.clone()), hash(second.clone()));
        let mut arena = Arena::new(100);
        let first_id = arena.function(first.clone()).unwrap();
        let second_id = arena.function(second.clone()).unwrap();
        assert_ne!(first_id, second_id);
        assert_eq!(arena.function(first.clone()).unwrap(), first_id);
        assert_eq!(arena.function(second.clone()).unwrap(), second_id);
        assert_eq!(arena.entries(first_id).unwrap(), first);
        assert_eq!(arena.entries(second_id).unwrap(), second);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn collisions_in_an_overlay_keep_base_and_local_values_distinct() {
        use crate::safety_work::WorkBudget;
        let first = vec![
            (Scalar::Int(0), Scalar::Int(-1_370_170_012)),
            (Scalar::Int(95_947_124), Scalar::Int(0)),
        ];
        let second = vec![
            (Scalar::Int(0), Scalar::Int(803_057_848)),
            (Scalar::Int(1_091_714_232), Scalar::Int(-1_935_938_920)),
        ];
        let mut arena = Arena::new(1000);
        let base = arena.function(first.clone()).unwrap();
        let budget = WorkBudget::new(900, 1);
        let mut overlay = arena.fork(&budget, 0);
        let local = overlay.function(second.clone()).unwrap();
        assert_ne!(base, local);
        assert_eq!(overlay.function(first.clone()).unwrap(), base);
        let mapping = arena.merge(overlay.into_delta()).unwrap();
        assert_eq!(arena.entries(base).unwrap(), first);
        assert_eq!(arena.entries(mapping.get(local)).unwrap(), second);
        assert_ne!(base, mapping.get(local));
        assert_eq!(arena.function(second).unwrap(), mapping.get(local));
    }

    #[test]
    fn replacement_keeps_the_normalized_domain_and_canonical_identity() {
        let mut arena = Arena::new(1000);
        let original = arena
            .function(vec![
                (Scalar::Int(3), Scalar::Int(30)),
                (Scalar::Int(1), Scalar::Int(10)),
                (Scalar::Int(2), Scalar::Int(20)),
            ])
            .unwrap();
        let updated = arena
            .replace(original, &[Scalar::Int(2)], Scalar::Int(21))
            .unwrap();
        let expected = arena
            .function(vec![
                (Scalar::Int(2), Scalar::Int(21)),
                (Scalar::Int(1), Scalar::Int(10)),
                (Scalar::Int(3), Scalar::Int(30)),
            ])
            .unwrap();
        assert_eq!(updated, expected);
        assert_eq!(arena.at(original, Scalar::Int(2)).unwrap(), Scalar::Int(20));
        assert_eq!(
            arena
                .replace(original, &[Scalar::Int(4)], Scalar::Int(40))
                .unwrap(),
            original
        );
        assert_eq!(
            arena
                .replace(updated, &[Scalar::Int(2)], Scalar::Int(20))
                .unwrap(),
            original
        );
    }
    #[test]
    fn integer_positions_preserve_sparse_domains_and_missing_keys() {
        let mut arena = Arena::new(1000);
        let dense = arena
            .function(
                [4, 1, 3, 2]
                    .into_iter()
                    .map(|key| (Scalar::Int(key), Scalar::Int(key * 10)))
                    .collect(),
            )
            .unwrap();
        for key in 1..=4 {
            assert_eq!(
                arena.at(dense, Scalar::Int(key)).unwrap(),
                Scalar::Int(key * 10)
            );
        }
        let sparse = arena
            .function(
                [-3, 0, 2, 10]
                    .into_iter()
                    .map(|key| (Scalar::Int(key), Scalar::Int(key * 10)))
                    .collect(),
            )
            .unwrap();
        for key in [-3, 0, 2, 10] {
            assert_eq!(
                arena.at(sparse, Scalar::Int(key)).unwrap(),
                Scalar::Int(key * 10)
            );
        }
        for key in [i32::MIN, -1, 1, 3, i32::MAX] {
            assert_eq!(arena.select(sparse, Scalar::Int(key)).unwrap(), None);
        }
        let updated = arena
            .replace(sparse, &[Scalar::Int(2)], Scalar::Int(77))
            .unwrap();
        assert_eq!(arena.at(updated, Scalar::Int(2)).unwrap(), Scalar::Int(77));
        assert_eq!(arena.at(updated, Scalar::Int(0)).unwrap(), Scalar::Int(0));
        assert_eq!(arena.at(sparse, Scalar::Int(2)).unwrap(), Scalar::Int(20));
        assert_eq!(
            arena
                .replace(updated, &[Scalar::Int(3)], Scalar::Int(99))
                .unwrap(),
            updated
        );
        let extreme = arena
            .function(vec![
                (Scalar::Int(i32::MIN), Scalar::Int(7)),
                (Scalar::Int(i32::MAX), Scalar::Int(9)),
            ])
            .unwrap();
        assert_eq!(
            arena.at(extreme, Scalar::Int(i32::MIN)).unwrap(),
            Scalar::Int(7)
        );
        assert_eq!(
            arena.at(extreme, Scalar::Int(i32::MAX)).unwrap(),
            Scalar::Int(9)
        );
        let string = arena.string("1".into()).unwrap();
        let record = arena.function(vec![(string, Scalar::Bool(true))]).unwrap();
        assert_eq!(arena.at(record, string).unwrap(), Scalar::Bool(true));
        assert_eq!(arena.select(record, Scalar::Int(1)).unwrap(), None);
        assert!(arena.at(record, Scalar::Int(1)).is_err());
        let boolean = arena
            .function(vec![
                (Scalar::Bool(false), Scalar::Int(3)),
                (Scalar::Bool(true), Scalar::Int(4)),
            ])
            .unwrap();
        assert_eq!(
            arena.at(boolean, Scalar::Bool(true)).unwrap(),
            Scalar::Int(4)
        );
        assert_eq!(arena.select(boolean, Scalar::Int(1)).unwrap(), None);
    }
}
