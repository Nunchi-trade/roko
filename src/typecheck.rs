#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Int(i64),
    Bool(bool),
    Add(Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    Mismatch { expected: Type, found: Type },
    BranchesMismatch { then_ty: Type, else_ty: Type },
}

pub fn typecheck(expr: &Expr) -> Result<Type, TypeError> {
    match expr {
        Expr::Int(_) => Ok(Type::Int),
        Expr::Bool(_) => Ok(Type::Bool),
        Expr::Add(left, right) => {
            let left_ty = typecheck(left)?;
            let right_ty = typecheck(right)?;
            ensure_type(Type::Int, left_ty)?;
            ensure_type(Type::Int, right_ty)?;
            Ok(Type::Int)
        }
        Expr::If(cond, then_branch, else_branch) => {
            let cond_ty = typecheck(cond)?;
            ensure_type(Type::Bool, cond_ty)?;

            let then_ty = typecheck(then_branch)?;
            let else_ty = typecheck(else_branch)?;
            if then_ty == else_ty {
                Ok(then_ty)
            } else {
                Err(TypeError::BranchesMismatch { then_ty, else_ty })
            }
        }
    }
}

fn ensure_type(expected: Type, found: Type) -> Result<(), TypeError> {
    if expected == found {
        Ok(())
    } else {
        Err(TypeError::Mismatch { expected, found })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_literal_has_int_type() {
        assert_eq!(typecheck(&Expr::Int(1)), Ok(Type::Int));
    }

    #[test]
    fn bool_literal_has_bool_type() {
        assert_eq!(typecheck(&Expr::Bool(true)), Ok(Type::Bool));
    }

    #[test]
    fn add_of_ints_has_int_type() {
        let expr = Expr::Add(Box::new(Expr::Int(1)), Box::new(Expr::Int(2)));
        assert_eq!(typecheck(&expr), Ok(Type::Int));
    }

    #[test]
    fn add_rejects_bool_operand() {
        let expr = Expr::Add(Box::new(Expr::Bool(true)), Box::new(Expr::Int(2)));
        assert_eq!(
            typecheck(&expr),
            Err(TypeError::Mismatch {
                expected: Type::Int,
                found: Type::Bool,
            })
        );
    }

    #[test]
    fn if_expression_requires_bool_condition() {
        let expr = Expr::If(
            Box::new(Expr::Int(0)),
            Box::new(Expr::Int(1)),
            Box::new(Expr::Int(2)),
        );
        assert_eq!(
            typecheck(&expr),
            Err(TypeError::Mismatch {
                expected: Type::Bool,
                found: Type::Int,
            })
        );
    }

    #[test]
    fn if_expression_returns_branch_type() {
        let expr = Expr::If(
            Box::new(Expr::Bool(true)),
            Box::new(Expr::Int(1)),
            Box::new(Expr::Int(2)),
        );
        assert_eq!(typecheck(&expr), Ok(Type::Int));
    }

    #[test]
    fn if_expression_rejects_mismatched_branches() {
        let expr = Expr::If(
            Box::new(Expr::Bool(true)),
            Box::new(Expr::Int(1)),
            Box::new(Expr::Bool(false)),
        );
        assert_eq!(
            typecheck(&expr),
            Err(TypeError::BranchesMismatch {
                then_ty: Type::Int,
                else_ty: Type::Bool,
            })
        );
    }
}
