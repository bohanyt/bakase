use rusqlite::{
    Connection, Error,
    functions::{Aggregate, Context, FunctionFlags, WindowAggregate},
    types::{Value, ValueRef},
};

pub(super) const NON_NUMERIC: &str = "hitung_xlsx_non_numeric:";
const OVERFLOW: &str = "hitung_xlsx_overflow";

#[derive(Clone, Copy)]
enum Numeric {
    Sum,
    Average,
    Total,
}

#[derive(Default)]
struct Accumulator {
    integers: i128,
    integer_count: i64,
    approximate: f64,
    correction: f64,
    real_count: i64,
    bad: i64,
}

impl Accumulator {
    fn count(&self) -> i64 {
        self.integer_count + self.real_count
    }

    fn add_compensated(&mut self, value: f64) {
        let sum = self.approximate;
        let next = sum + value;
        self.correction += if sum.abs() > value.abs() {
            (sum - next) + value
        } else {
            (value - next) + sum
        };
        self.approximate = next;
    }

    fn change(&mut self, input: ValueRef<'_>, sign: i64) {
        match input {
            ValueRef::Null => {}
            ValueRef::Integer(n) => {
                let value = i128::from(n) * i128::from(sign);
                self.integers += value;
                self.integer_count += sign;
                if value.unsigned_abs() >= (1_u128 << 52) {
                    // SQLite memisahkan bit rendah sebelum mengubah bilangan besar ke f64.
                    let low = value % 16_384;
                    self.add_compensated((value - low) as f64);
                    self.add_compensated(low as f64);
                } else {
                    self.add_compensated(value as f64);
                }
            }
            ValueRef::Real(n) => {
                self.real_count += sign;
                self.add_compensated(n * sign as f64);
            }
            _ => self.bad += sign,
        }
        if self.count() == 0 {
            self.approximate = 0.0;
            self.correction = 0.0;
        }
    }
}

impl Numeric {
    fn result(self, acc: &Accumulator) -> rusqlite::Result<Value> {
        if acc.bad != 0 {
            return Err(Error::UserFunctionError(
                format!("{NON_NUMERIC}{}", acc.bad).into(),
            ));
        }
        if acc.count() == 0 && !matches!(self, Self::Total) {
            return Ok(Value::Null);
        }
        if acc.real_count == 0 && matches!(self, Self::Sum) {
            return i64::try_from(acc.integers)
                .map(Value::Integer)
                .map_err(|_| Error::UserFunctionError(OVERFLOW.into()));
        }
        let total = if acc.real_count == 0 {
            acc.integers as f64
        } else {
            acc.approximate + acc.correction
        };
        let value = if matches!(self, Self::Average) {
            total / acc.count() as f64
        } else {
            total
        };
        if !value.is_finite() {
            return Err(Error::UserFunctionError(OVERFLOW.into()));
        }
        Ok(Value::Real(value))
    }
}

impl Aggregate<Accumulator, Value> for Numeric {
    fn init(&self, _ctx: &mut Context<'_>) -> rusqlite::Result<Accumulator> {
        Ok(Accumulator::default())
    }

    fn step(&self, ctx: &mut Context<'_>, acc: &mut Accumulator) -> rusqlite::Result<()> {
        acc.change(ctx.get_raw(0), 1);
        Ok(())
    }

    fn finalize(
        &self,
        _ctx: &mut Context<'_>,
        acc: Option<Accumulator>,
    ) -> rusqlite::Result<Value> {
        self.result(&acc.unwrap_or_default())
    }
}

impl WindowAggregate<Accumulator, Value> for Numeric {
    fn value(&self, acc: Option<&mut Accumulator>) -> rusqlite::Result<Value> {
        self.result(acc.as_deref().unwrap_or(&Accumulator::default()))
    }

    fn inverse(&self, ctx: &mut Context<'_>, acc: &mut Accumulator) -> rusqlite::Result<()> {
        acc.change(ctx.get_raw(0), -1);
        Ok(())
    }
}

pub(super) fn register(db: &Connection) -> rusqlite::Result<()> {
    for (name, function) in [
        ("sum", Numeric::Sum),
        ("avg", Numeric::Average),
        ("total", Numeric::Total),
    ] {
        db.create_window_function(
            name,
            1,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            function,
        )?;
    }
    Ok(())
}

pub(super) fn failure(error: &Error) -> Option<Result<i64, &'static str>> {
    let message = match error {
        Error::SqliteFailure(_, Some(message)) => message.as_str(),
        _ => return None,
    };
    if message == OVERFLOW {
        return Some(Err(
            "Hasil agregat di luar batas angka. Perkecil skala atau hitung per kelompok.",
        ));
    }
    Some(Ok(message.strip_prefix(NON_NUMERIC)?.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_aggregates_match_sqlite_with_cancellation_and_sliding_windows() {
        let builtin = Connection::open_in_memory().unwrap();
        let strict = Connection::open_in_memory().unwrap();
        register(&strict).unwrap();
        let data = "WITH x(n,v) AS (VALUES
            (1,9007199254740993),(2,-9007199254740992.0),(3,0.25),(4,2),(5,3))";
        let read = |db: &Connection, sql: &str| {
            db.prepare(&format!("{data} {sql}"))
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, Value>(0)?,
                        row.get::<_, Value>(1)?,
                        row.get::<_, Value>(2)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let totals = "SELECT SUM(v), AVG(v), TOTAL(v) FROM x WHERE n <= 3";
        assert_eq!(
            read(&strict, totals),
            vec![(
                Value::Real(1.25),
                Value::Real(1.25 / 3.0),
                Value::Real(1.25)
            )]
        );
        assert_eq!(read(&strict, totals), read(&builtin, totals));
        let window = "SELECT SUM(v) OVER w, AVG(v) OVER w, TOTAL(v) OVER w
            FROM x WINDOW w AS (ORDER BY n ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)";
        let actual = read(&strict, window);
        let expected = read(&builtin, window);
        assert_eq!(&actual[..4], &expected[..4]);
        assert_eq!(
            actual[4],
            (Value::Integer(5), Value::Real(2.5), Value::Real(5.0))
        );
    }
}
