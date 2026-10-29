//! Evaluation of FEEL expressions and unary tests (DMN 1.5 §10.3.2). Every error is a
//! value: an operation that is not defined for its operands gives `null`.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use super::builtins;
use super::num::Num;
use super::parse::{Expr, Tests, Type};
use super::temporal::{self, AnyDuration, Date, DateTime, DtDuration, Time, YmDuration, Zone};
use super::{Func, Map, Param, Range, Value, compare, equal};

/// A scope of variables; inner scopes see the outer ones.
#[derive(Debug, Default)]
pub struct Env {
    pub vars: Map,
    pub parent: Option<Rc<Env>>,
}

impl Env {
    pub fn root(vars: Map) -> Rc<Env> {
        Rc::new(Env { vars, parent: None })
    }
    pub fn child(parent: &Rc<Env>, vars: Map) -> Rc<Env> {
        Rc::new(Env {
            vars,
            parent: Some(parent.clone()),
        })
    }
    pub fn lookup(&self, name: &str) -> Option<&Value> {
        let mut e = self;
        loop {
            if let Some(v) = e.vars.get(name) {
                return Some(v);
            }
            e = e.parent.as_ref()?;
        }
    }
}

/// An item definition (DMN `itemDefinition`): a base type or components, a collection
/// flag and allowed values.
#[derive(Debug, Clone, Default)]
pub struct TypeDef {
    pub base: Option<Type>,
    pub collection: bool,
    pub allowed: Option<Tests>,
    pub components: Vec<(String, TypeDef)>,
}

/// The item definitions of a model by name.
#[derive(Debug, Clone, Default)]
pub struct Types {
    pub defs: HashMap<String, TypeDef>,
}

/// The evaluator: the types of the model and the current time (`now()`, `today()`).
pub struct Eval<'a> {
    pub types: &'a Types,
    pub now: DateTime,
}

fn tri(v: Option<bool>) -> Value {
    v.map(Value::Bool).unwrap_or(Value::Null)
}

fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

/// A temporal value of an `@"…"` literal or a conversion of text.
pub fn temporal_literal(s: &str) -> Value {
    if s.starts_with('P') || s.starts_with("-P") {
        return match temporal::parse_duration(s) {
            Some(AnyDuration::Dt(d)) => Value::Dt(d),
            Some(AnyDuration::Ym(d)) => Value::Ym(d),
            None => Value::Null,
        };
    }
    if s.contains('T') {
        return DateTime::parse(s)
            .map(Value::DateTime)
            .unwrap_or(Value::Null);
    }
    if s.len() >= 8 && s.as_bytes().get(2) == Some(&b':') {
        return Time::parse(s).map(Value::Time).unwrap_or(Value::Null);
    }
    Date::parse(s).map(Value::Date).unwrap_or(Value::Null)
}

/// The current time in UTC.
pub fn now_utc() -> DateTime {
    let nanos = crate::pytime::unix_nanos();
    DateTime::from_local(nanos / 1_000_000 * 1_000_000, Zone::Offset(0)).unwrap_or(DateTime {
        date: Date {
            year: 1970,
            month: 1,
            day: 1,
        },
        time: Time {
            nanos: 0,
            zone: Zone::Offset(0),
        },
    })
}

fn dt_times(d: &DtDuration, n: &Num) -> Option<DtDuration> {
    let x = Num::from_i128(d.0)?.mul(n)?;
    let r = x.round(0, crate::pydec::Rounding::HalfEven)?;
    let t = r.decimal().to_i128().ok()?;
    Some(DtDuration(t))
}

fn ym_times(d: &YmDuration, n: &Num) -> Option<YmDuration> {
    let x = Num::int(d.0).mul(n)?;
    Some(YmDuration(x.trunc_i64()?))
}

pub fn add(a: &Value, b: &Value) -> Value {
    use Value as V;
    match (a, b) {
        (V::Num(x), V::Num(y)) => Value::num(x.add(y)),
        (V::Str(x), V::Str(y)) => V::Str(format!("{x}{y}")),
        (V::Dt(x), V::Dt(y)) => {
            x.0.checked_add(y.0)
                .map(|n| V::Dt(DtDuration(n)))
                .unwrap_or(V::Null)
        }
        (V::Ym(x), V::Ym(y)) => {
            x.0.checked_add(y.0)
                .map(|n| V::Ym(YmDuration(n)))
                .unwrap_or(V::Null)
        }
        (V::DateTime(t), V::Dt(d)) | (V::Dt(d), V::DateTime(t)) => {
            t.add(d).map(V::DateTime).unwrap_or(V::Null)
        }
        (V::DateTime(t), V::Ym(d)) | (V::Ym(d), V::DateTime(t)) => {
            t.add_months(d.0).map(V::DateTime).unwrap_or(V::Null)
        }
        (V::Date(t), V::Dt(d)) | (V::Dt(d), V::Date(t)) => DateTime::of_date(t)
            .add(d)
            .map(|x| V::Date(x.date))
            .unwrap_or(V::Null),
        (V::Date(t), V::Ym(d)) | (V::Ym(d), V::Date(t)) => {
            t.add_months(d.0).map(V::Date).unwrap_or(V::Null)
        }
        (V::Time(t), V::Dt(d)) | (V::Dt(d), V::Time(t)) => V::Time(t.add(d)),
        _ => V::Null,
    }
}

fn as_datetime(v: &Value) -> Option<DateTime> {
    match v {
        Value::DateTime(d) => Some(d.clone()),
        Value::Date(d) => Some(DateTime {
            date: *d,
            time: Time {
                nanos: 0,
                zone: Zone::Offset(0),
            },
        }),
        _ => None,
    }
}

pub fn sub(a: &Value, b: &Value) -> Value {
    use Value as V;
    match (a, b) {
        (V::Num(x), V::Num(y)) => Value::num(x.sub(y)),
        (V::Dt(x), V::Dt(y)) => {
            x.0.checked_sub(y.0)
                .map(|n| V::Dt(DtDuration(n)))
                .unwrap_or(V::Null)
        }
        (V::Ym(x), V::Ym(y)) => {
            x.0.checked_sub(y.0)
                .map(|n| V::Ym(YmDuration(n)))
                .unwrap_or(V::Null)
        }
        // a date is the date and time of its midnight in UTC
        (V::Date(_) | V::DateTime(_), V::Date(_) | V::DateTime(_)) => {
            match (as_datetime(a), as_datetime(b)) {
                (Some(x), Some(y)) => x.sub(&y).map(V::Dt).unwrap_or(V::Null),
                _ => V::Null,
            }
        }
        (V::DateTime(t), V::Dt(d)) => t.add(&DtDuration(-d.0)).map(V::DateTime).unwrap_or(V::Null),
        (V::DateTime(t), V::Ym(d)) => t.add_months(-d.0).map(V::DateTime).unwrap_or(V::Null),
        (V::Date(t), V::Dt(d)) => DateTime::of_date(t)
            .add(&DtDuration(-d.0))
            .map(|x| V::Date(x.date))
            .unwrap_or(V::Null),
        (V::Date(t), V::Ym(d)) => t.add_months(-d.0).map(V::Date).unwrap_or(V::Null),
        (V::Time(t), V::Dt(d)) => V::Time(t.add(&DtDuration(-d.0))),
        (V::Time(x), V::Time(y)) => x.sub(y).map(V::Dt).unwrap_or(V::Null),
        _ => V::Null,
    }
}

pub fn mul(a: &Value, b: &Value) -> Value {
    use Value as V;
    match (a, b) {
        (V::Num(x), V::Num(y)) => Value::num(x.mul(y)),
        (V::Dt(d), V::Num(n)) | (V::Num(n), V::Dt(d)) => {
            dt_times(d, n).map(V::Dt).unwrap_or(V::Null)
        }
        (V::Ym(d), V::Num(n)) | (V::Num(n), V::Ym(d)) => {
            ym_times(d, n).map(V::Ym).unwrap_or(V::Null)
        }
        _ => V::Null,
    }
}

pub fn div(a: &Value, b: &Value) -> Value {
    use Value as V;
    match (a, b) {
        (V::Num(x), V::Num(y)) => Value::num(x.div(y)),
        (V::Dt(d), V::Num(n)) => {
            if n.is_zero() {
                return V::Null;
            }
            Num::int(1)
                .div(n)
                .and_then(|inv| dt_times(d, &inv))
                .map(V::Dt)
                .unwrap_or(V::Null)
        }
        (V::Ym(d), V::Num(n)) => {
            if n.is_zero() {
                return V::Null;
            }
            Num::int(d.0)
                .div(n)
                .and_then(|x| x.trunc_i64())
                .map(|m| V::Ym(YmDuration(m)))
                .unwrap_or(V::Null)
        }
        (V::Dt(x), V::Dt(y)) => match (Num::from_i128(x.0), Num::from_i128(y.0)) {
            (Some(p), Some(q)) => Value::num(p.div(&q)),
            _ => V::Null,
        },
        (V::Ym(x), V::Ym(y)) => Value::num(Num::int(x.0).div(&Num::int(y.0))),
        _ => V::Null,
    }
}

/// `a op b` for `<`, `<=`, `>`, `>=`, `=`, `!=`.
pub fn cmp_values(op: &str, a: &Value, b: &Value) -> Option<bool> {
    match op {
        "=" => equal(a, b),
        "!=" => equal(a, b).map(|x| !x),
        _ => {
            let o = compare(a, b)?;
            Some(match op {
                "<" => o == Ordering::Less,
                "<=" => o != Ordering::Greater,
                ">" => o == Ordering::Greater,
                ">=" => o != Ordering::Less,
                _ => return None,
            })
        }
    }
}

/// Whether `v` lies in the range.
pub fn in_range(v: &Value, r: &Range) -> Option<bool> {
    if v.is_null() {
        return None;
    }
    if let Some(lo) = &r.low {
        let o = compare(v, lo)?;
        if o == Ordering::Less || (o == Ordering::Equal && !r.low_closed) {
            return Some(false);
        }
    }
    if let Some(hi) = &r.high {
        let o = compare(v, hi)?;
        if o == Ordering::Greater || (o == Ordering::Equal && !r.high_closed) {
            return Some(false);
        }
    }
    Some(true)
}

fn mentions_q(e: &Expr) -> bool {
    let mut names = Vec::new();
    collect_names(e, &mut names);
    names.iter().any(|n| n == "?")
}

fn collect_names(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Name(n) => out.push(n.clone()),
        Expr::List(i) => i.iter().for_each(|x| collect_names(x, out)),
        Expr::Context(en) => en.iter().for_each(|(_, x)| collect_names(x, out)),
        Expr::Range(a, _, b, _) => [a, b]
            .into_iter()
            .flatten()
            .for_each(|x| collect_names(x, out)),
        Expr::Compare1(_, x) | Expr::Neg(x) | Expr::InstanceOf(x, _) | Expr::Path(x, _) => {
            collect_names(x, out)
        }
        Expr::Filter(a, b)
        | Expr::Arith(_, a, b)
        | Expr::Pow(a, b)
        | Expr::Cmp(_, a, b)
        | Expr::And(a, b)
        | Expr::Or(a, b) => {
            collect_names(a, out);
            collect_names(b, out);
        }
        Expr::Call(f, args) => {
            collect_names(f, out);
            args.iter().for_each(|(_, a)| collect_names(a, out));
        }
        Expr::Between(a, b, c) | Expr::If(a, b, c) => {
            collect_names(a, out);
            collect_names(b, out);
            collect_names(c, out);
        }
        Expr::In(a, t) => {
            collect_names(a, out);
            t.iter().for_each(|x| collect_names(x, out));
        }
        Expr::For(c, b) => {
            for (_, f, t) in c {
                collect_names(f, out);
                if let Some(t) = t {
                    collect_names(t, out);
                }
            }
            collect_names(b, out);
        }
        Expr::Quantified(_, c, b) => {
            c.iter().for_each(|(_, l)| collect_names(l, out));
            collect_names(b, out);
        }
        Expr::Function(_, b) => collect_names(b, out),
        _ => {}
    }
}

fn as_list(v: Value) -> Rc<Vec<Value>> {
    match v {
        Value::List(l) => l,
        Value::Null => Rc::new(Vec::new()),
        other => Rc::new(vec![other]),
    }
}

impl Eval<'_> {
    pub fn new(types: &Types) -> Eval<'_> {
        Eval {
            types,
            now: now_utc(),
        }
    }

    pub fn eval(&self, e: &Expr, env: &Rc<Env>) -> Value {
        use Value as V;
        match e {
            Expr::Null => V::Null,
            Expr::Bool(b) => V::Bool(*b),
            Expr::Num(n) => V::Num(n.clone()),
            Expr::Str(s) => V::Str(s.clone()),
            Expr::At(s) => temporal_literal(s),
            Expr::Name(n) => match env.lookup(n) {
                Some(v) => v.clone(),
                None => match builtins::canonical(n) {
                    Some(b) => V::Func(Rc::new(Func::Builtin(b))),
                    None => dotted(n, env),
                },
            },
            Expr::List(items) => V::list(items.iter().map(|x| self.eval(x, env)).collect()),
            Expr::Context(entries) => {
                let scope = Env::child(env, Map::new());
                let mut scope = scope;
                let mut m = Map::new();
                for (k, x) in entries {
                    let v = self.eval(x, &scope);
                    m.insert(k.clone(), v.clone());
                    let mut vars = Map::new();
                    vars.insert(k.clone(), v);
                    scope = Env::child(&scope, vars);
                }
                V::context(m)
            }
            Expr::Range(lo, lc, hi, hc) => {
                let low = lo.as_ref().map(|x| self.eval(x, env));
                let high = hi.as_ref().map(|x| self.eval(x, env));
                make_range(low, *lc, high, *hc)
            }
            Expr::Compare1(op, x) => {
                let v = self.eval(x, env);
                match *op {
                    "<" => make_range(None, false, Some(v), false),
                    "<=" => make_range(None, false, Some(v), true),
                    ">" => make_range(Some(v), false, None, false),
                    ">=" => make_range(Some(v), true, None, false),
                    _ => V::Null,
                }
            }
            Expr::Path(x, name) => path(&self.eval(x, env), name),
            Expr::Filter(x, f) => self.filter(self.eval(x, env), f, env),
            Expr::Call(f, args) => self.invoke(f, args, env),
            Expr::Neg(x) => match self.eval(x, env) {
                V::Num(n) => V::Num(n.neg()),
                V::Dt(d) => V::Dt(DtDuration(-d.0)),
                V::Ym(d) => V::Ym(YmDuration(-d.0)),
                _ => V::Null,
            },
            Expr::Arith(op, a, b) => {
                let (a, b) = (self.eval(a, env), self.eval(b, env));
                match op {
                    '+' => add(&a, &b),
                    '-' => sub(&a, &b),
                    '*' => mul(&a, &b),
                    _ => div(&a, &b),
                }
            }
            Expr::Pow(a, b) => match (self.eval(a, env), self.eval(b, env)) {
                (V::Num(x), V::Num(y)) => Value::num(x.pow(&y)),
                _ => V::Null,
            },
            Expr::Cmp(op, a, b) => tri(cmp_values(op, &self.eval(a, env), &self.eval(b, env))),
            Expr::Between(x, lo, hi) => {
                let v = self.eval(x, env);
                let a = cmp_values(">=", &v, &self.eval(lo, env));
                let b = cmp_values("<=", &v, &self.eval(hi, env));
                tri(and3(a, b))
            }
            Expr::In(x, tests) => {
                let v = self.eval(x, env);
                let mut unknown = false;
                for t in tests {
                    match self.positive(t, &v, env) {
                        Some(true) => return V::Bool(true),
                        Some(false) => {}
                        None => unknown = true,
                    }
                }
                if unknown { V::Null } else { V::Bool(false) }
            }
            Expr::InstanceOf(x, ty) => V::Bool(self.instance_of(&self.eval(x, env), ty)),
            Expr::And(a, b) => {
                let x = as_bool(&self.eval(a, env));
                if x == Some(false) {
                    return V::Bool(false);
                }
                tri(and3(x, as_bool(&self.eval(b, env))))
            }
            Expr::Or(a, b) => {
                let x = as_bool(&self.eval(a, env));
                if x == Some(true) {
                    return V::Bool(true);
                }
                tri(or3(x, as_bool(&self.eval(b, env))))
            }
            Expr::If(c, a, b) => {
                if as_bool(&self.eval(c, env)) == Some(true) {
                    self.eval(a, env)
                } else {
                    self.eval(b, env)
                }
            }
            Expr::For(ctx, body) => self.for_loop(ctx, body, env),
            Expr::Quantified(every, ctx, body) => {
                let mut found = false;
                let mut all = true;
                if !self.quantify(ctx, 0, env, body, *every, &mut found, &mut all) {
                    return V::Null;
                }
                if *every { V::Bool(all) } else { V::Bool(found) }
            }
            Expr::Function(params, body) => V::Func(Rc::new(Func::Lambda {
                params: params
                    .iter()
                    .map(|(n, t)| Param {
                        name: n.clone(),
                        ty: t.clone(),
                    })
                    .collect(),
                body: body.clone(),
                env: env.clone(),
            })),
        }
    }

    /// `some`/`every`: false when an iteration context is not a list.
    #[allow(clippy::too_many_arguments)]
    fn quantify(
        &self,
        ctx: &[(String, Expr)],
        k: usize,
        env: &Rc<Env>,
        body: &Expr,
        every: bool,
        found: &mut bool,
        all: &mut bool,
    ) -> bool {
        if (every && !*all) || (!every && *found) {
            return true;
        }
        if k == ctx.len() {
            // a result that is not a boolean makes the whole expression null
            match as_bool(&self.eval(body, env)) {
                Some(true) => *found = true,
                Some(false) => *all = false,
                None => return false,
            }
            return true;
        }
        let (name, l) = &ctx[k];
        let list = match self.eval(l, env) {
            Value::List(items) => items,
            Value::Null => return false,
            other => Rc::new(vec![other]),
        };
        for item in list.iter() {
            let mut vars = Map::new();
            vars.insert(name.clone(), item.clone());
            let scope = Env::child(env, vars);
            if !self.quantify(ctx, k + 1, &scope, body, every, found, all) {
                return false;
            }
        }
        true
    }

    /// The values of one iteration context (`list`, `a..b`, or a range).
    fn iteration(&self, from: &Expr, to: Option<&Expr>, env: &Rc<Env>) -> Option<Rc<Vec<Value>>> {
        let f = self.eval(from, env);
        Some(match to {
            Some(t) => Rc::new(iterate(&f, &self.eval(t, env))?),
            None => match &f {
                Value::Range(r) => match (&r.low, &r.high) {
                    (Some(a), Some(b)) => {
                        let mut l = iterate(a, b)?;
                        if !r.low_closed && !l.is_empty() {
                            l.remove(0);
                        }
                        if !r.high_closed {
                            l.pop();
                        }
                        Rc::new(l)
                    }
                    _ => return None,
                },
                Value::Null => return None,
                _ => as_list(f),
            },
        })
    }

    fn for_loop(&self, ctx: &[(String, Expr, Option<Expr>)], body: &Expr, env: &Rc<Env>) -> Value {
        let mut out: Vec<Value> = Vec::new();
        // `partial` (the results so far) is a copy of them in every iteration; it is made
        // only for a body that reads it, so that other iterations take linear time
        let mut names = Vec::new();
        collect_names(body, &mut names);
        let partial = names.iter().any(|n| n == "partial");
        if self.for_level(ctx, 0, env, body, partial, &mut out) {
            Value::list(out)
        } else {
            Value::Null
        }
    }

    fn for_level(
        &self,
        ctx: &[(String, Expr, Option<Expr>)],
        k: usize,
        env: &Rc<Env>,
        body: &Expr,
        partial: bool,
        out: &mut Vec<Value>,
    ) -> bool {
        if k == ctx.len() {
            if partial {
                let mut v = Map::new();
                v.insert("partial".into(), Value::list(out.clone()));
                let scope = Env::child(env, v);
                out.push(self.eval(body, &scope));
            } else {
                out.push(self.eval(body, env));
            }
            return true;
        }
        let (name, from, to) = &ctx[k];
        let Some(list) = self.iteration(from, to.as_ref(), env) else {
            return false;
        };
        for item in list.iter() {
            let mut vars = Map::new();
            vars.insert(name.clone(), item.clone());
            let scope = Env::child(env, vars);
            if !self.for_level(ctx, k + 1, &scope, body, partial, out) {
                return false;
            }
        }
        true
    }

    fn filter(&self, base: Value, f: &Expr, env: &Rc<Env>) -> Value {
        if base.is_null() {
            return Value::Null;
        }
        let list = as_list(base);
        // a number selects an element (1 is the first, -1 the last)
        if !mentions_item(f)
            && let Value::Num(n) = self.eval(f, env)
        {
            let Some(i) = n.to_i64() else {
                return Value::Null;
            };
            let len = list.len() as i64;
            let idx = if i > 0 { i - 1 } else { len + i };
            if i == 0 || idx < 0 || idx >= len {
                return Value::Null;
            }
            return list[idx as usize].clone();
        }
        let mut out = Vec::new();
        for item in list.iter() {
            let mut vars = Map::new();
            vars.insert("item".into(), item.clone());
            if let Value::Context(m) = item {
                for (k, v) in m.iter() {
                    vars.insert(k.clone(), v.clone());
                }
            }
            let scope = Env::child(env, vars);
            match self.eval(f, &scope) {
                Value::Bool(true) => out.push(item.clone()),
                Value::Num(_) => return Value::Null,
                _ => {}
            }
        }
        Value::list(out)
    }

    fn invoke(&self, f: &Expr, args: &[(Option<String>, Expr)], env: &Rc<Env>) -> Value {
        let callee = match f {
            Expr::Name(n) => match env.lookup(n) {
                Some(Value::Func(func)) => Some(func.clone()),
                Some(_) => None,
                None => builtins::canonical(n).map(|b| Rc::new(Func::Builtin(b))),
            },
            other => match self.eval(other, env) {
                Value::Func(func) => Some(func),
                _ => None,
            },
        };
        let Some(func) = callee else {
            return Value::Null;
        };
        let mut positional = Vec::new();
        let mut named = Vec::new();
        for (name, a) in args {
            let v = self.eval(a, env);
            match name {
                Some(n) => named.push((n.clone(), v)),
                None => positional.push(v),
            }
        }
        if !named.is_empty() && !positional.is_empty() {
            return Value::Null;
        }
        if named.is_empty() {
            self.call(&func, positional)
        } else {
            self.call_named(&func, named)
        }
    }

    /// Call a function with positional arguments.
    pub fn call(&self, f: &Func, args: Vec<Value>) -> Value {
        match f {
            Func::Builtin(name) => builtins::call(self, name, args),
            Func::Lambda { params, .. } | Func::Boxed { params, .. } => {
                if args.len() > params.len() {
                    return Value::Null;
                }
                let mut bound = Vec::new();
                for (k, p) in params.iter().enumerate() {
                    bound.push((p.name.clone(), args.get(k).cloned().unwrap_or(Value::Null)));
                }
                self.call_user(f, bound)
            }
        }
    }

    fn call_named(&self, f: &Func, named: Vec<(String, Value)>) -> Value {
        match f {
            Func::Builtin(name) => match builtins::by_name(name, &named) {
                Some(args) => builtins::call(self, name, args),
                None => Value::Null,
            },
            Func::Lambda { params, .. } | Func::Boxed { params, .. } => {
                if named
                    .iter()
                    .any(|(n, _)| !params.iter().any(|p| p.name == *n))
                {
                    return Value::Null;
                }
                let bound = params
                    .iter()
                    .map(|p| {
                        let v = named
                            .iter()
                            .find(|(n, _)| *n == p.name)
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null);
                        (p.name.clone(), v)
                    })
                    .collect();
                self.call_user(f, bound)
            }
        }
    }

    fn call_user(&self, f: &Func, bound: Vec<(String, Value)>) -> Value {
        let (params, env) = match f {
            Func::Lambda { params, env, .. } | Func::Boxed { params, env, .. } => (params, env),
            Func::Builtin(_) => return Value::Null,
        };
        let mut vars = Map::new();
        for ((name, v), p) in bound.into_iter().zip(params) {
            let v = match &p.ty {
                Some(t) => match self.coerce(v, t) {
                    Some(v) => v,
                    None => return Value::Null,
                },
                None => v,
            };
            vars.insert(name, v);
        }
        let scope = Env::child(env, vars);
        match f {
            Func::Lambda { body, .. } => self.eval(body, &scope),
            Func::Boxed { body, result, .. } => {
                let v = body.call(self, &scope);
                match result {
                    Some(t) => self.coerce(v, t).unwrap_or(Value::Null),
                    None => v,
                }
            }
            Func::Builtin(_) => Value::Null,
        }
    }

    /// A positive unary test applied to `input` (DMN 1.5 §10.3.2.10).
    pub fn positive(&self, t: &Expr, input: &Value, env: &Rc<Env>) -> Option<bool> {
        match t {
            Expr::Compare1(op, x) => {
                let v = self.eval(x, env);
                cmp_values(op, input, &v)
            }
            Expr::Range(..) => match self.eval(t, env) {
                Value::Range(r) => in_range(input, &r),
                _ => None,
            },
            _ => {
                let mut vars = Map::new();
                vars.insert("?".into(), input.clone());
                let scope = Env::child(env, vars);
                if mentions_q(t) {
                    return as_bool(&self.eval(t, &scope));
                }
                let v = self.eval(t, &scope);
                match &v {
                    Value::Range(r) => in_range(input, r),
                    Value::List(items) => {
                        if equal(input, &v) == Some(true) {
                            return Some(true);
                        }
                        Some(items.iter().any(|x| match x {
                            Value::Range(r) => in_range(input, r) == Some(true),
                            _ => equal(input, x) == Some(true),
                        }))
                    }
                    Value::Bool(b) if !matches!(input, Value::Bool(_)) => Some(*b),
                    _ => equal(input, &v),
                }
            }
        }
    }

    /// Unary tests applied to `input`: `true` when they match.
    pub fn test(&self, tests: &Tests, input: &Value, env: &Rc<Env>) -> bool {
        match tests {
            Tests::Any => true,
            Tests::Some(ts) => ts
                .iter()
                .any(|t| self.positive(t, input, env) == Some(true)),
            Tests::Not(ts) => {
                let mut all_false = true;
                for t in ts {
                    match self.positive(t, input, env) {
                        Some(false) => {}
                        _ => all_false = false,
                    }
                }
                all_false
            }
        }
    }

    /// `value instance of type`.
    pub fn instance_of(&self, v: &Value, t: &Type) -> bool {
        use Value as V;
        match t {
            Type::Any => !v.is_null(),
            Type::Null => v.is_null(),
            Type::Number => matches!(v, V::Num(_)),
            Type::String => matches!(v, V::Str(_)),
            Type::Boolean => matches!(v, V::Bool(_)),
            Type::Date => matches!(v, V::Date(_)),
            Type::Time => matches!(v, V::Time(_)),
            Type::DateTime => matches!(v, V::DateTime(_)),
            Type::DtDuration => matches!(v, V::Dt(_)),
            Type::YmDuration => matches!(v, V::Ym(_)),
            Type::List(inner) => match v {
                V::List(items) => items
                    .iter()
                    .all(|x| x.is_null() || self.instance_of(x, inner)),
                _ => false,
            },
            Type::Context(fields) => match v {
                V::Context(m) => fields.iter().all(|(k, ft)| match m.get(k) {
                    Some(x) => x.is_null() || self.instance_of(x, ft),
                    None => false,
                }),
                _ => false,
            },
            Type::Range(inner) => match v {
                V::Range(r) => [&r.low, &r.high]
                    .into_iter()
                    .flatten()
                    .all(|x| self.instance_of(x, inner)),
                _ => false,
            },
            Type::Function(params, _) => match v {
                V::Func(f) => match f.as_ref() {
                    Func::Builtin(_) => true,
                    Func::Lambda { params: ps, .. } | Func::Boxed { params: ps, .. } => {
                        params.is_empty()
                            || (ps.len() == params.len()
                                && ps.iter().zip(params).all(|(p, t)| match &p.ty {
                                    None => true,
                                    Some(pt) => subtype(t, pt) || matches!(t, Type::Any),
                                }))
                    }
                },
                _ => false,
            },
            Type::Named(n) => match self.types.defs.get(n) {
                Some(def) => self.conforms_def(v, def, true),
                None => false,
            },
        }
    }

    /// `strict`: `instance of` (every component present, allowed values not checked);
    /// otherwise the conformance of a value to the type of an element (missing
    /// components are null, allowed values are checked).
    fn conforms_def(&self, v: &Value, def: &TypeDef, strict: bool) -> bool {
        if def.collection {
            let Value::List(items) = v else {
                return false;
            };
            let single = TypeDef {
                collection: false,
                ..def.clone()
            };
            return items
                .iter()
                .all(|x| x.is_null() || self.conforms_def(x, &single, strict));
        }
        if let Some(b) = &def.base {
            let ok = match b {
                Type::Named(n) => match self.types.defs.get(n) {
                    Some(d) => self.conforms_def(v, d, strict),
                    None => false,
                },
                other => self.instance_of(v, other),
            };
            if !ok {
                return false;
            }
        }
        if !def.components.is_empty() {
            let Value::Context(m) = v else {
                return false;
            };
            for (k, cd) in &def.components {
                match m.get(k) {
                    Some(x) if !x.is_null() && !self.conforms_def(x, cd, strict) => return false,
                    None => return false,
                    _ => {}
                }
            }
        }
        if !strict
            && let Some(a) = &def.allowed
            && !self.test(a, v, &Env::root(Map::new()))
        {
            return false;
        }
        true
    }

    /// Whether a value conforms to the type of an element (allowed values checked).
    pub fn conforms(&self, v: &Value, t: &Type) -> bool {
        match t {
            Type::Named(n) => match self.types.defs.get(n) {
                Some(def) => self.conforms_def(v, def, false),
                None => false,
            },
            other => self.instance_of(v, other),
        }
    }

    /// A value converted to a type (DMN 1.5 §10.3.2.9.4): itself when it conforms, the
    /// element of a singleton list, a singleton list of a value, a date as a date and
    /// time; `None` when it does not conform.
    pub fn coerce(&self, v: Value, t: &Type) -> Option<Value> {
        if v.is_null() || matches!(t, Type::Any) {
            return Some(v);
        }
        if self.conforms(&v, t) {
            return Some(v);
        }
        let is_list_type = match t {
            Type::List(_) => true,
            Type::Named(n) => self.types.defs.get(n).is_some_and(|d| d.collection),
            _ => false,
        };
        if let Value::List(items) = &v
            && items.len() == 1
            && !is_list_type
        {
            // one level only: the element itself must conform
            let x = items[0].clone();
            if x.is_null() || self.conforms(&x, t) {
                return Some(x);
            }
            return match (&x, t) {
                (Value::Date(_), Type::DateTime) => self.coerce(x, t),
                _ => None,
            };
        }
        if is_list_type && !matches!(v, Value::List(_)) {
            let l = Value::list(vec![v]);
            return self.conforms(&l, t).then_some(l);
        }
        if let (Value::Date(d), Type::DateTime) = (&v, t) {
            return Some(Value::DateTime(DateTime {
                date: *d,
                time: Time {
                    nanos: 0,
                    zone: Zone::Offset(0),
                },
            }));
        }
        None
    }
}

/// A name with dots that is not a variable, read as a path: the longest prefix that is a
/// variable, then its fields.
fn dotted(n: &str, env: &Env) -> Value {
    let parts: Vec<&str> = n.split('.').collect();
    for k in (1..parts.len()).rev() {
        let head = parts[..k].join(".");
        if let Some(v) = env.lookup(head.trim()) {
            let mut cur = v.clone();
            for p in &parts[k..] {
                cur = path(&cur, p.trim());
            }
            return cur;
        }
    }
    Value::Null
}

fn subtype(a: &Type, b: &Type) -> bool {
    a == b || matches!(b, Type::Any)
}

fn mentions_item(e: &Expr) -> bool {
    let mut names = Vec::new();
    collect_names(e, &mut names);
    names.iter().any(|n| n == "item")
}

fn and3(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

fn or3(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

/// A range value; `null` when the ends are not comparable.
pub fn make_range(low: Option<Value>, lc: bool, high: Option<Value>, hc: bool) -> Value {
    if let (Some(a), Some(b)) = (&low, &high) {
        if a.is_null() && b.is_null() {
            return Value::Null;
        }
        if !a.is_null() && !b.is_null() && compare(a, b).is_none() {
            return Value::Null;
        }
    }
    let ok = |v: &Option<Value>| match v {
        None => true,
        Some(x) => compare(x, x).is_some() || x.is_null(),
    };
    if !ok(&low) || !ok(&high) {
        return Value::Null;
    }
    if let (Some(a), Some(b)) = (&low, &high)
        && compare(a, b) == Some(std::cmp::Ordering::Greater)
    {
        return Value::Null;
    }
    Value::Range(Rc::new(Range {
        low,
        low_closed: lc,
        high,
        high_closed: hc,
    }))
}

/// The values of an iteration context `a..b` (integers or dates, either direction).
fn iterate(a: &Value, b: &Value) -> Option<Vec<Value>> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => {
            let (x, y) = (x.to_i64()?, y.to_i64()?);
            if (y - x).abs() > 10_000_000 {
                return None;
            }
            Some(if x <= y {
                (x..=y).map(Value::int).collect()
            } else {
                (y..=x).rev().map(Value::int).collect()
            })
        }
        (Value::Date(x), Value::Date(y)) => {
            let (p, q) = (x.days(), y.days());
            if (q - p).abs() > 10_000_000 {
                return None;
            }
            let days: Vec<i64> = if p <= q {
                (p..=q).collect()
            } else {
                (q..=p).rev().collect()
            };
            Some(
                days.into_iter()
                    .filter_map(Date::from_days)
                    .map(Value::Date)
                    .collect(),
            )
        }
        _ => None,
    }
}

/// `value.name`: a context entry, the same path over a list, or a property of a
/// temporal value or a range.
pub fn path(v: &Value, name: &str) -> Value {
    use Value as V;
    let secs = |t: &Time| -> Value {
        let n = Num::int(t.second());
        if t.subsec() == 0 {
            return V::Num(n);
        }
        Value::num(Num::parse(&format!("{}.{:09}", t.second(), t.subsec())))
    };
    let offset = |z: &Zone| match z {
        Zone::Offset(o) => V::Dt(DtDuration(i128::from(*o) * 1_000_000_000)),
        Zone::Id(_) => z
            .offset()
            .map(|o| V::Dt(DtDuration(i128::from(o) * 1_000_000_000)))
            .unwrap_or(V::Null),
        Zone::None => V::Null,
    };
    let zone = |z: &Zone| match z {
        Zone::Id(id) => V::Str(id.clone()),
        _ => V::Null,
    };
    let date_prop = |d: &Date| -> Option<Value> {
        Some(match name {
            "year" => V::int(d.year),
            "month" => V::int(i64::from(d.month)),
            "day" => V::int(i64::from(d.day)),
            "weekday" => V::int(d.weekday()),
            _ => return None,
        })
    };
    let time_prop = |t: &Time| -> Option<Value> {
        Some(match name {
            "hour" => V::int(t.hour()),
            "minute" => V::int(t.minute()),
            "second" => secs(t),
            "time offset" => offset(&t.zone),
            "timezone" => zone(&t.zone),
            _ => return None,
        })
    };
    match v {
        V::Context(m) => m.get(name).cloned().unwrap_or(V::Null),
        V::List(items) => V::list(items.iter().map(|x| path(x, name)).collect()),
        V::Date(d) => date_prop(d).unwrap_or(V::Null),
        V::Time(t) => time_prop(t).unwrap_or(V::Null),
        V::DateTime(dt) => date_prop(&dt.date)
            .or_else(|| time_prop(&dt.time))
            .unwrap_or(V::Null),
        V::Dt(d) => match name {
            "days" => Value::num(Num::from_i128(d.days())),
            "hours" => Value::num(Num::from_i128(d.hours())),
            "minutes" => Value::num(Num::from_i128(d.minutes())),
            "seconds" => {
                let (s, f) = d.seconds();
                if f == 0 {
                    Value::num(Num::from_i128(s))
                } else {
                    let sign = if s < 0 || f < 0 { "-" } else { "" };
                    Value::num(Num::parse(&format!("{sign}{}.{:09}", s.abs(), f.abs())))
                }
            }
            _ => V::Null,
        },
        V::Ym(d) => match name {
            "years" => V::int(d.0 / 12),
            "months" => V::int(d.0 % 12),
            _ => V::Null,
        },
        V::Range(r) => match name {
            "start" => r.low.clone().unwrap_or(V::Null),
            "end" => r.high.clone().unwrap_or(V::Null),
            "start included" => V::Bool(r.low.is_some() && r.low_closed),
            "end included" => V::Bool(r.high.is_some() && r.high_closed),
            _ => V::Null,
        },
        _ => V::Null,
    }
}

pub(super) fn bool_of(v: &Value) -> Option<bool> {
    as_bool(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feel3::parse::{Names, parse_expression};

    fn ev(text: &str) -> String {
        let types = Types::default();
        let e = Eval::new(&types);
        let names = Names::builtins();
        match parse_expression(text, &names) {
            Ok(x) => e.eval(&x, &Env::root(Map::new())).literal(),
            Err(err) => format!("error: {err:?}"),
        }
    }

    #[test]
    fn expressions() {
        assert_eq!(ev("1 + 2 * 3"), "7");
        assert_eq!(ev("\"a\" + \"b\""), "\"ab\"");
        assert_eq!(ev("[1, 2, 3][item > 1]"), "[2, 3]");
        assert_eq!(ev("[1, 2, 3][-1]"), "3");
        assert_eq!(ev("{a: 1, b: a + 1}.b"), "2");
        assert_eq!(ev("for x in 1..3 return x * x"), "[1, 4, 9]");
        assert_eq!(ev("some x in [1, 5] satisfies x > 3"), "true");
        assert_eq!(ev("every x in [1, 5] satisfies x > 3"), "false");
        assert_eq!(ev("5 in (< 3, > 4)"), "true");
        assert_eq!(ev("5 between 1 and 10"), "true");
        assert_eq!(
            ev("date(\"2027-01-31\") + duration(\"P1M\")"),
            "@\"2027-02-28\""
        );
        assert_eq!(
            ev("date and time(\"2027-01-01T10:00:00\") - date and time(\"2026-12-31T09:30:00\")"),
            "@\"P1DT30M\""
        );
        assert_eq!(ev("(function(a, b) a * b)(2, 3)"), "6");
        assert_eq!(ev("1 = \"1\""), "null");
        assert_eq!(ev("null = null"), "true");
        assert_eq!(ev("[1..5] = [1..5]"), "true");
        assert_eq!(ev("@\"2027-01-15\".weekday"), "5");
        assert_eq!(ev("if 1 > 2 then \"a\" else \"b\""), "\"b\"");
        assert_eq!(ev("3 instance of number"), "true");
        assert_eq!(ev("[1, 2] instance of list<number>"), "true");
    }
}
