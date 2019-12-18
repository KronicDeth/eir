use libeir_util_number::Integer;

use libeir_diagnostics::ByteSpan;

use crate::lexer::{symbols, Ident, Symbol};
use crate::parser::ast::*;

use super::errors::PreprocessorError;

/// This evaluator is used for performing simple reductions
/// during preprocessing, namely for evaluating conditionals
/// in -if/-elseif directives.
///
/// As a result, the output of this function is _not_ a primitive
/// value, but rather an Expr which has been reduced to its simplest
/// form (e.g. a BinaryOp that can be evaluated at compile-time would
/// be converted into the corresponding literal representation of the
/// result of that op)
///
/// Exprs which are not able to be evaluated at compile-time will be
/// treated as errors. In particular the following constructs are supported,
/// and you can consider everything else as invalid unless explicitly noted:
///
/// - Math on constants or expressions which evaluate to constants
/// - Bit shift operations on constants or expressions which evaluate to constants
/// - Comparisons on constants or expressions which evaluate to constants
/// - The use of `++` and `--` on constant lists, or expressions which evaluate to constant lists
pub fn eval(expr: Expr) -> Result<Expr, PreprocessorError> {
    let result = match expr {
        // Nothing to be done here
        Expr::Var(_) => expr,
        Expr::Literal(_) => expr,
        Expr::Nil(_) => expr,
        Expr::FunctionName(_) => expr,
        Expr::RecordIndex(_) => expr,

        // Recursively evaluate subexpressions
        Expr::Cons(Cons {
            span,
            id,
            head,
            tail,
        }) => Expr::Cons(Cons {
            span,
            id,
            head: Box::new(eval(*head)?),
            tail: Box::new(eval(*tail)?),
        }),
        Expr::Tuple(Tuple { span, id, elements }) => Expr::Tuple(Tuple {
            span,
            id,
            elements: eval_list(elements)?,
        }),
        Expr::Map(Map { span, id, fields }) => Expr::Map(Map {
            span,
            id,
            fields: eval_map(fields)?,
        }),
        Expr::MapUpdate(MapUpdate {
            span,
            id,
            map,
            updates,
        }) => Expr::MapUpdate(MapUpdate {
            span,
            id,
            map: Box::new(eval(*map)?),
            updates: eval_map(updates)?,
        }),
        Expr::MapProjection(MapProjection {
            span,
            id,
            map,
            fields,
        }) => Expr::MapProjection(MapProjection {
            span,
            id,
            map: Box::new(eval(*map)?),
            fields: eval_map(fields)?,
        }),
        Expr::Binary(Binary { span, id, elements }) => Expr::Binary(Binary {
            span,
            id,
            elements: eval_bin_elements(elements)?,
        }),
        Expr::Record(Record { span, id, name, fields }) => Expr::Record(Record {
            span,
            id,
            name,
            fields: eval_record(fields)?,
        }),
        Expr::RecordAccess(RecordAccess {
            span,
            id,
            record,
            name,
            field,
        }) => Expr::RecordAccess(RecordAccess {
            span,
            id,
            record: Box::new(eval(*record)?),
            name,
            field,
        }),
        Expr::RecordUpdate(RecordUpdate {
            span,
            id,
            record,
            name,
            updates,
        }) => Expr::RecordUpdate(RecordUpdate {
            span,
            id,
            record: Box::new(eval(*record)?),
            name,
            updates: eval_record(updates)?,
        }),
        Expr::Begin(Begin { span, .. }) => {
            return Err(PreprocessorError::InvalidConstExpression { span });
        }
        Expr::Apply(Apply {
            span,
            callee,
            args,
            ..
        }) => {
            let _args = eval_list(args)?;
            match eval(*callee)? {
                Expr::Literal(Literal::Atom(_, Ident { ref name, .. })) => match builtin(*name) {
                    None => {
                        return Err(PreprocessorError::InvalidConstExpression { span });
                    }
                    Some(_) => unimplemented!(),
                },
                _ => return Err(PreprocessorError::InvalidConstExpression { span }),
            }
        }
        Expr::BinaryExpr(BinaryExpr {
            span,
            id,
            lhs,
            op,
            rhs,
        }) => {
            let lhs = eval(*lhs)?;
            let rhs = eval(*rhs)?;
            return eval_binary_op(span, id, lhs, op, rhs);
        }
        Expr::UnaryExpr(UnaryExpr {
            span,
            id: _,
            op,
            operand,
        }) => {
            let operand = eval(*operand)?;
            return eval_unary_op(span, op, operand);
        }
        expr => {
            return Err(PreprocessorError::InvalidConstExpression {
                span: expr.span(),
            });
        }
    };

    Ok(result)
}

fn eval_list(mut exprs: Vec<Expr>) -> Result<Vec<Expr>, PreprocessorError> {
    let mut result = Vec::new();

    for expr in exprs.drain(..) {
        result.push(eval(expr)?);
    }

    Ok(result)
}

fn eval_map(mut fields: Vec<MapField>) -> Result<Vec<MapField>, PreprocessorError> {
    let mut result = Vec::new();

    for field in fields.drain(..) {
        match field {
            MapField::Assoc { span, id, key, value } => result.push(MapField::Assoc {
                span,
                id,
                key: eval(key)?,
                value: eval(value)?,
            }),
            MapField::Exact { span, id, key, value } => result.push(MapField::Exact {
                span,
                id,
                key: eval(key)?,
                value: eval(value)?,
            }),
        }
    }

    Ok(result)
}

fn eval_record(mut fields: Vec<RecordField>) -> Result<Vec<RecordField>, PreprocessorError> {
    let mut result = Vec::new();

    for field in fields.drain(..) {
        let new_field = match field {
            RecordField {
                span,
                id,
                name,
                value: Some(value),
                ty,
            } => RecordField {
                span,
                id,
                name,
                value: Some(eval(value)?),
                ty,
            },
            RecordField {
                span,
                id,
                name,
                value: None,
                ty,
            } => RecordField {
                span,
                id,
                name,
                value: None,
                ty,
            },
        };
        result.push(new_field);
    }

    Ok(result)
}

fn eval_bin_elements(
    mut elements: Vec<BinaryElement>,
) -> Result<Vec<BinaryElement>, PreprocessorError> {
    let mut result = Vec::new();

    for element in elements.drain(..) {
        let new_element = match element {
            BinaryElement {
                span,
                id,
                bit_expr,
                bit_size: Some(bit_size),
                bit_type,
            } => BinaryElement {
                span,
                id,
                bit_expr: eval(bit_expr)?,
                bit_size: Some(eval(bit_size)?),
                bit_type,
            },

            BinaryElement {
                span,
                id,
                bit_expr,
                bit_size: None,
                bit_type,
            } => BinaryElement {
                span,
                id,
                bit_expr: eval(bit_expr)?,
                bit_size: None,
                bit_type,
            },
        };

        result.push(new_element);
    }

    Ok(result)
}

fn eval_binary_op(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    match op {
        BinaryOp::OrElse | BinaryOp::AndAlso | BinaryOp::Or | BinaryOp::And => {
            eval_boolean(span, id, lhs, op, rhs)
        }
        BinaryOp::Equal | BinaryOp::NotEqual => eval_equality(span, id, lhs, op, rhs),
        BinaryOp::StrictEqual | BinaryOp::StrictNotEqual => {
            eval_strict_equality(span, id, lhs, op, rhs)
        }
        BinaryOp::Lte | BinaryOp::Lt | BinaryOp::Gte | BinaryOp::Gt => {
            eval_comparison(span, id, lhs, op, rhs)
        }
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Div
        | BinaryOp::Rem => eval_arith(span, id, lhs, op, rhs),
        BinaryOp::Bor
        | BinaryOp::Bxor
        | BinaryOp::Xor
        | BinaryOp::Band
        | BinaryOp::Bsl
        | BinaryOp::Bsr => eval_shift(span, id, lhs, op, rhs),
        _ => return Err(PreprocessorError::InvalidConstExpression { span }),
    }
}

fn eval_unary_op(span: ByteSpan, op: UnaryOp, rhs: Expr) -> Result<Expr, PreprocessorError> {
    let expr = match op {
        UnaryOp::Plus => match rhs {
            Expr::Literal(Literal::Integer(id, span, i)) if i < 0 => {
                Expr::Literal(Literal::Integer(id, span, -i))
            }
            Expr::Literal(Literal::Integer(_, _, _)) => rhs,
            Expr::Literal(Literal::Float(id, span, i)) if i < 0.0 => {
                Expr::Literal(Literal::Float(id, span, i * -1.0))
            }
            Expr::Literal(Literal::Float(_, _, _)) => rhs,
            _ => return Err(PreprocessorError::InvalidConstExpression { span }),
        },
        UnaryOp::Minus => match rhs {
            Expr::Literal(Literal::Integer(id, span, i)) if i > 0 => {
                Expr::Literal(Literal::Integer(id, span, -i))
            }
            Expr::Literal(Literal::Integer(_, _, _)) => rhs,
            Expr::Literal(Literal::Float(id, span, i)) if i > 0.0 => {
                Expr::Literal(Literal::Float(id, span, i * -1.0))
            }
            Expr::Literal(Literal::Float(_, _, _)) => rhs,
            _ => return Err(PreprocessorError::InvalidConstExpression { span }),
        },
        UnaryOp::Bnot => match rhs {
            Expr::Literal(Literal::Integer(id, span, Integer::Small(i))) => Expr::Literal(Literal::Integer(id, span, (!i).into())),
            _ => return Err(PreprocessorError::InvalidConstExpression { span }),
        },
        UnaryOp::Not => match rhs {
            Expr::Literal(Literal::Atom(id, Ident { name, span })) if name == symbols::True => {
                Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                }))
            }
            Expr::Literal(Literal::Atom(id, Ident { name, span })) if name == symbols::False => {
                Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                }))
            }
            _ => return Err(PreprocessorError::InvalidConstExpression { span }),
        },
    };
    Ok(expr)
}

fn eval_boolean(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    if !is_boolean(&lhs) || !is_boolean(&rhs) {
        return Err(PreprocessorError::InvalidConstExpression { span });
    }
    let left = is_true(&lhs);
    let right = is_true(&rhs);

    match op {
        BinaryOp::Xor => {
            if (left != right) && (left || right) {
                return Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })));
            }
            return Ok(Expr::Literal(Literal::Atom(id, Ident {
                name: symbols::False,
                span,
            })));
        }
        BinaryOp::OrElse | BinaryOp::Or => {
            if left || right {
                return Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })));
            } else {
                return Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })));
            }
        }
        BinaryOp::AndAlso | BinaryOp::And => {
            if left && right {
                return Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })));
            } else {
                return Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })));
            }
        }
        _ => unreachable!(),
    }
}

fn eval_equality(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    if is_number(&lhs) && is_number(&rhs) {
        eval_numeric_equality(span, id, lhs, op, rhs)
    } else {
        match op {
            BinaryOp::Equal => {
                if lhs == rhs {
                    Ok(Expr::Literal(Literal::Atom(id, Ident {
                        name: symbols::True,
                        span,
                    })))
                } else {
                    Ok(Expr::Literal(Literal::Atom(id, Ident {
                        name: symbols::False,
                        span,
                    })))
                }
            }
            BinaryOp::NotEqual => {
                if lhs != rhs {
                    Ok(Expr::Literal(Literal::Atom(id, Ident {
                        name: symbols::True,
                        span,
                    })))
                } else {
                    Ok(Expr::Literal(Literal::Atom(id, Ident {
                        name: symbols::False,
                        span,
                    })))
                }
            }
            _ => unreachable!(),
        }
    }
}

fn eval_strict_equality(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    match op {
        BinaryOp::StrictEqual => {
            if lhs == rhs {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })))
            } else {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })))
            }
        }
        BinaryOp::StrictNotEqual => {
            if lhs != rhs {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })))
            } else {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })))
            }
        }
        _ => unreachable!(),
    }
}

fn eval_numeric_equality(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    let result = match (lhs, rhs) {
        (Expr::Literal(Literal::Integer(_, _, x)), Expr::Literal(Literal::Integer(_, _, y))) => {
            match op {
                BinaryOp::Equal if x == y => Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })),
                BinaryOp::NotEqual if x != y => Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })),
                BinaryOp::Equal => Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })),
                BinaryOp::NotEqual => Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })),
                _ => unreachable!(),
            }
        }
        (Expr::Literal(Literal::Float(_, _, x)), Expr::Literal(Literal::Float(_, _, y))) => match op {
            BinaryOp::Equal if x == y => Expr::Literal(Literal::Atom(id, Ident {
                name: symbols::True,
                span,
            })),
            BinaryOp::NotEqual if x != y => Expr::Literal(Literal::Atom(id, Ident {
                name: symbols::True,
                span,
            })),
            BinaryOp::Equal => Expr::Literal(Literal::Atom(id, Ident {
                name: symbols::False,
                span,
            })),
            BinaryOp::NotEqual => Expr::Literal(Literal::Atom(id, Ident {
                name: symbols::False,
                span,
            })),
            _ => unreachable!(),
        },

        (
            Expr::Literal(Literal::Integer(xspan, xid, x)),
            rhs @ Expr::Literal(Literal::Float(_, _, _)),
        ) => {
            return eval_numeric_equality(
                span,
                id,
                Expr::Literal(Literal::Float(xspan, xid, x.to_float())),
                op,
                rhs,
            );
        }

        (
            lhs @ Expr::Literal(Literal::Float(_, _, _)),
            Expr::Literal(Literal::Integer(yspan, yid, y)),
        ) => {
            return eval_numeric_equality(
                span,
                id,
                lhs,
                op,
                Expr::Literal(Literal::Float(yspan, yid, y.to_float())),
            );
        }

        _ => return Err(PreprocessorError::InvalidConstExpression { span }),
    };

    Ok(result)
}

fn eval_comparison(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    match op {
        BinaryOp::Lt | BinaryOp::Lte => {
            if lhs < rhs {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })))
            } else if op == BinaryOp::Lte {
                eval_equality(span, id, lhs, BinaryOp::Equal, rhs)
            } else {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })))
            }
        }
        BinaryOp::Gt | BinaryOp::Gte => {
            if lhs > rhs {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::True,
                    span,
                })))
            } else if op == BinaryOp::Gte {
                eval_equality(span, id, lhs, BinaryOp::Equal, rhs)
            } else {
                Ok(Expr::Literal(Literal::Atom(id, Ident {
                    name: symbols::False,
                    span,
                })))
            }
        }
        _ => unreachable!(),
    }
}

fn eval_arith(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    if is_number(&lhs) && is_number(&rhs) {
        let result = match (lhs, rhs) {
            // Types match
            (Expr::Literal(Literal::Integer(_, _, x)), Expr::Literal(Literal::Integer(_, _, y))) => {
                eval_op_int(span, id, x, op, &y)?
            }
            (Expr::Literal(Literal::Float(_, _, x)), Expr::Literal(Literal::Float(_, _, y))) => {
                eval_op_float(span, id, x, op, y)?
            }

            // Coerce to float
            (Expr::Literal(Literal::Integer(_, _, x)), Expr::Literal(Literal::Float(_, _, y))) => {
                eval_op_float(span, id, x.to_float(), op, y)?
            }
            (Expr::Literal(Literal::Float(_, _, x)), Expr::Literal(Literal::Integer(_, _, y))) => {
                eval_op_float(span, id, x, op, y.to_float())?
            }

            _ => return Err(PreprocessorError::InvalidConstExpression { span }),
        };
        Ok(result)
    } else {
        return Err(PreprocessorError::InvalidConstExpression { span });
    }
}

fn eval_op_int(
    span: ByteSpan,
    id: NodeId,
    x: Integer,
    op: BinaryOp,
    y: &Integer
) -> Result<Expr, PreprocessorError> {
    let result = match op {
        BinaryOp::Add => Expr::Literal(Literal::Integer(span, id, x + y)),
        BinaryOp::Sub => Expr::Literal(Literal::Integer(span, id, x - y)),
        BinaryOp::Multiply => Expr::Literal(Literal::Integer(span, id, x * y)),
        BinaryOp::Divide if *y == 0 => return Err(PreprocessorError::InvalidConstExpression{ span }),
        BinaryOp::Divide => Expr::Literal(Literal::Float(span, id, x.to_float() / y.to_float())),
        BinaryOp::Div if *y == 0 => return Err(PreprocessorError::InvalidConstExpression { span }),
        BinaryOp::Div => Expr::Literal(Literal::Integer(span, id, x / y)),
        BinaryOp::Rem => Expr::Literal(Literal::Integer(span, id, x % y)),
        _ => unreachable!(),
    };
    Ok(result)
}

fn eval_op_float(
    span: ByteSpan,
    id: NodeId,
    x: f64,
    op: BinaryOp,
    y: f64
) -> Result<Expr, PreprocessorError> {
    match op {
        BinaryOp::Add => Ok(Expr::Literal(Literal::Float(span, id, x + y))),
        BinaryOp::Sub => Ok(Expr::Literal(Literal::Float(span, id, x - y))),
        BinaryOp::Multiply => Ok(Expr::Literal(Literal::Float(span, id, x * y))),
        BinaryOp::Divide if y == 0.0 => {
            return Err(PreprocessorError::InvalidConstExpression { span })
        }
        BinaryOp::Divide => Ok(Expr::Literal(Literal::Float(span, id, x / y))),
        BinaryOp::Div => return Err(PreprocessorError::InvalidConstExpression { span }),
        BinaryOp::Rem => return Err(PreprocessorError::InvalidConstExpression { span }),
        _ => unreachable!(),
    }
}

fn eval_shift(
    span: ByteSpan,
    id: NodeId,
    lhs: Expr,
    op: BinaryOp,
    rhs: Expr,
) -> Result<Expr, PreprocessorError> {
    match (lhs, rhs) {
        (Expr::Literal(Literal::Integer(_, _, x)), Expr::Literal(Literal::Integer(_, _, y))) => {
            match (x, y) {
                (Integer::Small(x), Integer::Small(y)) => {
                    let result = match op {
                        BinaryOp::Bor => x | y,
                        BinaryOp::Bxor => x ^ y,
                        BinaryOp::Band => x & y,
                        BinaryOp::Bsl => x << y,
                        BinaryOp::Bsr => x >> y,
                        _ => unreachable!(),
                    };
                    Ok(Expr::Literal(Literal::Integer(span, id, result.into())))
                },
                _ => return Err(PreprocessorError::InvalidConstExpression { span }),
            }
        }
        _ => return Err(PreprocessorError::InvalidConstExpression { span }),
    }
}

fn is_number(e: &Expr) -> bool {
    match *e {
        Expr::Literal(Literal::Integer(_, _, _)) => true,
        Expr::Literal(Literal::Float(_, _, _)) => true,
        _ => false,
    }
}

fn is_boolean(e: &Expr) -> bool {
    match *e {
        Expr::Literal(Literal::Atom(_, Ident { ref name, .. })) => {
            if *name == symbols::True || *name == symbols::False {
                return true;
            }
            false
        }
        _ => false,
    }
}

fn is_true(e: &Expr) -> bool {
    match *e {
        Expr::Literal(Literal::Atom(_, Ident { ref name, .. })) if *name == symbols::True => true,
        _ => false,
    }
}

fn builtin(_name: Symbol) -> Option<&'static fn(Vec<Expr>) -> Result<Expr, ()>> {
    None
}
