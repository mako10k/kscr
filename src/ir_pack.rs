use crate::ir::{CastTarget, IrCaseArm, IrExpr, IrItem, IrLiteral, IrModule, IrPattern};

/// Maximum allowed length for collections to prevent excessive memory allocation
const MAX_COLLECTION_SIZE: u32 = 100_000_000; // 100M elements

/// Safely convert u32 length to usize with validation
fn validate_length(len: u32) -> Result<usize, String> {
    if len > MAX_COLLECTION_SIZE {
        return Err(format!(
            "collection size {} exceeds maximum allowed size {}",
            len, MAX_COLLECTION_SIZE
        ));
    }
    Ok(len as usize)
}

pub fn encode_ir_module(module: &IrModule) -> Vec<u8> {
    let mut out = Vec::new();
    write_u32(&mut out, module.items.len() as u32);
    for item in &module.items {
        encode_item(&mut out, item);
    }
    out
}

pub fn decode_ir_module(mut input: &[u8]) -> Result<IrModule, String> {
    let len = validate_length(read_u32(&mut input)?)?;
    let mut items = Vec::with_capacity(len);
    for _ in 0..len {
        items.push(decode_item(&mut input)?);
    }
    if !input.is_empty() {
        return Err("trailing bytes in packed IR".to_string());
    }
    Ok(IrModule { items })
}

fn encode_item(out: &mut Vec<u8>, item: &IrItem) {
    match item {
        IrItem::Binding { name, expr } => {
            write_u8(out, 0);
            write_string(out, name);
            encode_expr(out, expr);
        }
    }
}

fn decode_item(input: &mut &[u8]) -> Result<IrItem, String> {
    let tag = read_u8(input)?;
    match tag {
        0 => {
            let name = read_string(input)?;
            let expr = decode_expr(input)?;
            Ok(IrItem::Binding { name, expr })
        }
        other => Err(format!("unknown IrItem tag: {other}")),
    }
}

fn encode_literal(out: &mut Vec<u8>, lit: &IrLiteral) {
    match lit {
        IrLiteral::Unit => write_u8(out, 0),
        IrLiteral::Integer(s) => {
            write_u8(out, 1);
            write_string(out, s);
        }
        IrLiteral::Float64(s) => {
            write_u8(out, 2);
            write_string(out, s);
        }
        IrLiteral::Bool(b) => {
            write_u8(out, 3);
            write_u8(out, if *b { 1 } else { 0 });
        }
        IrLiteral::String(s) => {
            write_u8(out, 4);
            write_string(out, s);
        }
        IrLiteral::Char(c) => {
            write_u8(out, 5);
            write_u32(out, *c as u32);
        }
    }
}

fn decode_literal(input: &mut &[u8]) -> Result<IrLiteral, String> {
    let tag = read_u8(input)?;
    match tag {
        0 => Ok(IrLiteral::Unit),
        1 => Ok(IrLiteral::Integer(read_string(input)?)),
        2 => Ok(IrLiteral::Float64(read_string(input)?)),
        3 => {
            let b = read_u8(input)?;
            match b {
                0 => Ok(IrLiteral::Bool(false)),
                1 => Ok(IrLiteral::Bool(true)),
                _ => Err("invalid bool byte".to_string()),
            }
        }
        4 => Ok(IrLiteral::String(read_string(input)?)),
        5 => {
            let v = read_u32(input)?;
            std::char::from_u32(v)
                .map(IrLiteral::Char)
                .ok_or_else(|| "invalid char".to_string())
        }
        other => Err(format!("unknown IrLiteral tag: {other}")),
    }
}

fn encode_pattern(out: &mut Vec<u8>, pat: &IrPattern) {
    match pat {
        IrPattern::Var(s) => {
            write_u8(out, 0);
            write_string(out, s);
        }
        IrPattern::Wildcard => write_u8(out, 1),
        IrPattern::Literal(lit) => {
            write_u8(out, 2);
            encode_literal(out, lit);
        }
        IrPattern::Tuple(ps) => {
            write_u8(out, 3);
            write_u32(out, ps.len() as u32);
            for p in ps {
                encode_pattern(out, p);
            }
        }
        IrPattern::List(ps) => {
            write_u8(out, 4);
            write_u32(out, ps.len() as u32);
            for p in ps {
                encode_pattern(out, p);
            }
        }
        IrPattern::Record(fields) => {
            write_u8(out, 5);
            write_u32(out, fields.len() as u32);
            for (k, v) in fields {
                write_string(out, k);
                encode_pattern(out, v);
            }
        }
        IrPattern::RecordLoose(fields, tail) => {
            write_u8(out, 6);
            write_u32(out, fields.len() as u32);
            for (k, v) in fields {
                write_string(out, k);
                encode_pattern(out, v);
            }
            encode_opt_string(out, tail);
        }
        IrPattern::Cons(h, t) => {
            write_u8(out, 7);
            encode_pattern(out, h);
            encode_pattern(out, t);
        }
        IrPattern::Constructor { name, args } => {
            write_u8(out, 8);
            write_string(out, name);
            write_u32(out, args.len() as u32);
            for a in args {
                encode_pattern(out, a);
            }
        }
        IrPattern::Or(a, b) => {
            write_u8(out, 9);
            encode_pattern(out, a);
            encode_pattern(out, b);
        }
        IrPattern::As(name, p) => {
            write_u8(out, 10);
            write_string(out, name);
            encode_pattern(out, p);
        }
        IrPattern::View(pat, expr) => {
            write_u8(out, 11);
            encode_pattern(out, pat);
            encode_expr(out, expr);
        }
    }
}

fn decode_pattern(input: &mut &[u8]) -> Result<IrPattern, String> {
    let tag = read_u8(input)?;
    match tag {
        0 => Ok(IrPattern::Var(read_string(input)?)),
        1 => Ok(IrPattern::Wildcard),
        2 => Ok(IrPattern::Literal(decode_literal(input)?)),
        3 => {
            let n = validate_length(read_u32(input)?)?;
            let mut ps = Vec::with_capacity(n);
            for _ in 0..n {
                ps.push(decode_pattern(input)?);
            }
            Ok(IrPattern::Tuple(ps))
        }
        4 => {
            let n = validate_length(read_u32(input)?)?;
            let mut ps = Vec::with_capacity(n);
            for _ in 0..n {
                ps.push(decode_pattern(input)?);
            }
            Ok(IrPattern::List(ps))
        }
        5 => {
            let n = validate_length(read_u32(input)?)?;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let k = read_string(input)?;
                let v = decode_pattern(input)?;
                fields.push((k, v));
            }
            Ok(IrPattern::Record(fields))
        }
        6 => {
            let n = validate_length(read_u32(input)?)?;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let k = read_string(input)?;
                let v = decode_pattern(input)?;
                fields.push((k, v));
            }
            let tail = decode_opt_string(input)?;
            Ok(IrPattern::RecordLoose(fields, tail))
        }
        7 => {
            let h = decode_pattern(input)?;
            let t = decode_pattern(input)?;
            Ok(IrPattern::Cons(Box::new(h), Box::new(t)))
        }
        8 => {
            let name = read_string(input)?;
            let n = validate_length(read_u32(input)?)?;
            let mut args = Vec::with_capacity(n);
            for _ in 0..n {
                args.push(decode_pattern(input)?);
            }
            Ok(IrPattern::Constructor { name, args })
        }
        9 => {
            let a = decode_pattern(input)?;
            let b = decode_pattern(input)?;
            Ok(IrPattern::Or(Box::new(a), Box::new(b)))
        }
        10 => {
            let name = read_string(input)?;
            let p = decode_pattern(input)?;
            Ok(IrPattern::As(name, Box::new(p)))
        }
        11 => {
            let pat = decode_pattern(input)?;
            let expr = decode_expr(input)?;
            Ok(IrPattern::View(Box::new(pat), Box::new(expr)))
        }
        other => Err(format!("unknown IrPattern tag: {other}")),
    }
}

fn encode_case_arm(out: &mut Vec<u8>, arm: &IrCaseArm) {
    encode_pattern(out, &arm.pat);
    encode_opt_expr(out, &arm.guard);
    encode_expr(out, &arm.body);
}

fn decode_case_arm(input: &mut &[u8]) -> Result<IrCaseArm, String> {
    let pat = decode_pattern(input)?;
    let guard = decode_opt_expr(input)?;
    let body = decode_expr(input)?;
    Ok(IrCaseArm { pat, guard, body })
}

fn encode_cast_target(out: &mut Vec<u8>, t: CastTarget) {
    let tag = match t {
        CastTarget::I32 => 0,
        CastTarget::I64 => 1,
        CastTarget::F32 => 2,
        CastTarget::F64 => 3,
    };
    write_u8(out, tag);
}

fn decode_cast_target(input: &mut &[u8]) -> Result<CastTarget, String> {
    match read_u8(input)? {
        0 => Ok(CastTarget::I32),
        1 => Ok(CastTarget::I64),
        2 => Ok(CastTarget::F32),
        3 => Ok(CastTarget::F64),
        other => Err(format!("unknown CastTarget tag: {other}")),
    }
}

fn encode_expr(out: &mut Vec<u8>, expr: &IrExpr) {
    match expr {
        IrExpr::Unit => write_u8(out, 0),
        IrExpr::Integer(s) => {
            write_u8(out, 1);
            write_string(out, s);
        }
        IrExpr::Float64(s) => {
            write_u8(out, 2);
            write_string(out, s);
        }
        IrExpr::Bool(b) => {
            write_u8(out, 3);
            write_u8(out, if *b { 1 } else { 0 });
        }
        IrExpr::String(s) => {
            write_u8(out, 4);
            write_string(out, s);
        }
        IrExpr::Char(c) => {
            write_u8(out, 5);
            write_u32(out, *c as u32);
        }
        IrExpr::Var(s) => {
            write_u8(out, 6);
            write_string(out, s);
        }
        IrExpr::Lambda { params, body } => {
            write_u8(out, 7);
            write_u32(out, params.len() as u32);
            for p in params {
                write_string(out, p);
            }
            encode_expr(out, body);
        }
        IrExpr::Apply { func, args } => {
            write_u8(out, 8);
            encode_expr(out, func);
            write_u32(out, args.len() as u32);
            for a in args {
                encode_expr(out, a);
            }
        }
        IrExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            write_u8(out, 9);
            encode_expr(out, cond);
            encode_expr(out, then_branch);
            encode_expr(out, else_branch);
        }
        IrExpr::Let { bindings, body } => {
            write_u8(out, 10);
            write_u32(out, bindings.len() as u32);
            for (name, rhs) in bindings {
                write_string(out, name);
                encode_expr(out, rhs);
            }
            encode_expr(out, body);
        }
        IrExpr::Case { expr, arms } => {
            write_u8(out, 11);
            encode_expr(out, expr);
            write_u32(out, arms.len() as u32);
            for a in arms {
                encode_case_arm(out, a);
            }
        }
        IrExpr::IoBind {
            action,
            param,
            body,
        } => {
            write_u8(out, 12);
            encode_expr(out, action);
            write_string(out, param);
            encode_expr(out, body);
        }
        IrExpr::IoThen { first, then_expr } => {
            write_u8(out, 13);
            encode_expr(out, first);
            encode_expr(out, then_expr);
        }
        IrExpr::Cons { head, tail } => {
            write_u8(out, 14);
            encode_expr(out, head);
            encode_expr(out, tail);
        }
        IrExpr::List(xs) => {
            write_u8(out, 15);
            write_u32(out, xs.len() as u32);
            for x in xs {
                encode_expr(out, x);
            }
        }
        IrExpr::Tuple(xs) => {
            write_u8(out, 16);
            write_u32(out, xs.len() as u32);
            for x in xs {
                encode_expr(out, x);
            }
        }
        IrExpr::Record(fields) => {
            write_u8(out, 17);
            write_u32(out, fields.len() as u32);
            for (k, v) in fields {
                write_string(out, k);
                encode_expr(out, v);
            }
        }
        IrExpr::CheckedCast { expr, target } => {
            write_u8(out, 18);
            encode_expr(out, expr);
            encode_cast_target(out, *target);
        }
    }
}

fn decode_expr(input: &mut &[u8]) -> Result<IrExpr, String> {
    let tag = read_u8(input)?;
    match tag {
        0 => Ok(IrExpr::Unit),
        1 => Ok(IrExpr::Integer(read_string(input)?)),
        2 => Ok(IrExpr::Float64(read_string(input)?)),
        3 => {
            let b = read_u8(input)?;
            match b {
                0 => Ok(IrExpr::Bool(false)),
                1 => Ok(IrExpr::Bool(true)),
                _ => Err("invalid bool byte".to_string()),
            }
        }
        4 => Ok(IrExpr::String(read_string(input)?)),
        5 => {
            let v = read_u32(input)?;
            std::char::from_u32(v)
                .map(IrExpr::Char)
                .ok_or_else(|| "invalid char".to_string())
        }
        6 => Ok(IrExpr::Var(read_string(input)?)),
        7 => {
            let n = validate_length(read_u32(input)?)?;
            let mut params = Vec::with_capacity(n);
            for _ in 0..n {
                params.push(read_string(input)?);
            }
            let body = decode_expr(input)?;
            Ok(IrExpr::Lambda {
                params,
                body: Box::new(body),
            })
        }
        8 => {
            let func = decode_expr(input)?;
            let n = validate_length(read_u32(input)?)?;
            let mut args = Vec::with_capacity(n);
            for _ in 0..n {
                args.push(decode_expr(input)?);
            }
            Ok(IrExpr::Apply {
                func: Box::new(func),
                args,
            })
        }
        9 => {
            let cond = decode_expr(input)?;
            let then_branch = decode_expr(input)?;
            let else_branch = decode_expr(input)?;
            Ok(IrExpr::If {
                cond: Box::new(cond),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            })
        }
        10 => {
            let n = validate_length(read_u32(input)?)?;
            let mut bindings = Vec::with_capacity(n);
            for _ in 0..n {
                let name = read_string(input)?;
                let rhs = decode_expr(input)?;
                bindings.push((name, rhs));
            }
            let body = decode_expr(input)?;
            Ok(IrExpr::Let {
                bindings,
                body: Box::new(body),
            })
        }
        11 => {
            let expr = decode_expr(input)?;
            let n = validate_length(read_u32(input)?)?;
            let mut arms = Vec::with_capacity(n);
            for _ in 0..n {
                arms.push(decode_case_arm(input)?);
            }
            Ok(IrExpr::Case {
                expr: Box::new(expr),
                arms,
            })
        }
        12 => {
            let action = decode_expr(input)?;
            let param = read_string(input)?;
            let body = decode_expr(input)?;
            Ok(IrExpr::IoBind {
                action: Box::new(action),
                param,
                body: Box::new(body),
            })
        }
        13 => {
            let first = decode_expr(input)?;
            let then_expr = decode_expr(input)?;
            Ok(IrExpr::IoThen {
                first: Box::new(first),
                then_expr: Box::new(then_expr),
            })
        }
        14 => {
            let head = decode_expr(input)?;
            let tail = decode_expr(input)?;
            Ok(IrExpr::Cons {
                head: Box::new(head),
                tail: Box::new(tail),
            })
        }
        15 => {
            let n = validate_length(read_u32(input)?)?;
            let mut xs = Vec::with_capacity(n);
            for _ in 0..n {
                xs.push(decode_expr(input)?);
            }
            Ok(IrExpr::List(xs))
        }
        16 => {
            let n = validate_length(read_u32(input)?)?;
            let mut xs = Vec::with_capacity(n);
            for _ in 0..n {
                xs.push(decode_expr(input)?);
            }
            Ok(IrExpr::Tuple(xs))
        }
        17 => {
            let n = validate_length(read_u32(input)?)?;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let k = read_string(input)?;
                let v = decode_expr(input)?;
                fields.push((k, v));
            }
            Ok(IrExpr::Record(fields))
        }
        18 => {
            let expr = decode_expr(input)?;
            let target = decode_cast_target(input)?;
            Ok(IrExpr::CheckedCast {
                expr: Box::new(expr),
                target,
            })
        }
        other => Err(format!("unknown IrExpr tag: {other}")),
    }
}

fn encode_opt_string(out: &mut Vec<u8>, s: &Option<String>) {
    match s {
        None => write_u8(out, 0),
        Some(v) => {
            write_u8(out, 1);
            write_string(out, v);
        }
    }
}

fn decode_opt_string(input: &mut &[u8]) -> Result<Option<String>, String> {
    match read_u8(input)? {
        0 => Ok(None),
        1 => Ok(Some(read_string(input)?)),
        _ => Err("invalid Option<String> tag".to_string()),
    }
}

fn encode_opt_expr(out: &mut Vec<u8>, e: &Option<IrExpr>) {
    match e {
        None => write_u8(out, 0),
        Some(v) => {
            write_u8(out, 1);
            encode_expr(out, v);
        }
    }
}

fn decode_opt_expr(input: &mut &[u8]) -> Result<Option<IrExpr>, String> {
    match read_u8(input)? {
        0 => Ok(None),
        1 => Ok(Some(decode_expr(input)?)),
        _ => Err("invalid Option<IrExpr> tag".to_string()),
    }
}

fn write_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn read_u8(input: &mut &[u8]) -> Result<u8, String> {
    if input.is_empty() {
        return Err("unexpected EOF".to_string());
    }
    let v = input[0];
    *input = &input[1..];
    Ok(v)
}

fn read_u32(input: &mut &[u8]) -> Result<u32, String> {
    if input.len() < 4 {
        return Err("unexpected EOF".to_string());
    }
    let (a, rest) = input.split_at(4);
    *input = rest;
    Ok(u32::from_le_bytes([a[0], a[1], a[2], a[3]]))
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    write_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn read_string(input: &mut &[u8]) -> Result<String, String> {
    let len = validate_length(read_u32(input)?)?;
    if input.len() < len {
        return Err("unexpected EOF".to_string());
    }
    let (a, rest) = input.split_at(len);
    *input = rest;
    std::str::from_utf8(a)
        .map(|s| s.to_string())
        .map_err(|_| "invalid utf8 in string".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_small_module() {
        let m = IrModule {
            items: vec![IrItem::Binding {
                name: "main".to_string(),
                expr: IrExpr::Apply {
                    func: Box::new(IrExpr::Var("id".to_string())),
                    args: vec![IrExpr::Integer("1".to_string())],
                },
            }],
        };

        let bytes = encode_ir_module(&m);
        let m2 = decode_ir_module(&bytes).unwrap();
        assert_eq!(m, m2);
    }
}
