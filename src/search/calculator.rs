#[derive(Clone, Debug, PartialEq)]
pub struct CalcResult {
    pub value: f64,
    pub display: String,
    pub copy: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

#[derive(Clone, Debug)]
enum Expr {
    Number(f64),
    Percent(Box<Expr>),
    Negate(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Mod,
    Caret,
    Percent,
    LParen,
    RParen,
    Comma,
}

pub fn try_evaluate(query: &str) -> Option<CalcResult> {
    let query = query.trim();
    if query.is_empty() || query.len() > 512 {
        return None;
    }
    let tokens = tokenize(query)?;
    if tokens.is_empty() || !has_arithmetic_intent(&tokens) {
        return None;
    }

    let mut parser = Parser {
        tokens,
        position: 0,
    };
    let expr = parser.parse_expression()?;
    if parser.position != parser.tokens.len() {
        return None;
    }

    let value = evaluate(&expr)?;
    if !value.is_finite() {
        return None;
    }

    let copy = format_plain(value);
    Some(CalcResult {
        value,
        display: group_thousands(&copy),
        copy,
    })
}

pub fn is_expression(query: &str) -> bool {
    try_evaluate(query).is_some()
}

fn is_thousands_separator(chars: &[char], comma: usize) -> bool {
    let mut digits = 0;
    let mut index = comma + 1;
    while index < chars.len() && chars[index].is_ascii_digit() {
        digits += 1;
        index += 1;
    }
    if digits != 3 {
        return false;
    }
    match chars.get(index) {
        None => true,
        Some(next) => !next.is_ascii_digit() && *next != '_',
    }
}

fn has_arithmetic_intent(tokens: &[Token]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token,
            Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash
                | Token::Mod
                | Token::Caret
                | Token::Percent
                | Token::LParen
                | Token::Comma
        )
    }) || tokens
        .iter()
        .any(|token| matches!(token, Token::Ident(name) if is_function(name) || is_constant(name)))
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        let ch = chars[index];
        if ch.is_whitespace() {
            index += 1;
            continue;
        }

        if ch.is_ascii_digit()
            || (ch == '.' && chars.get(index + 1).is_some_and(char::is_ascii_digit))
        {
            let start = index;
            let mut seen_dot = false;
            while index < chars.len() {
                let current = chars[index];
                if current.is_ascii_digit() || current == '_' {
                    index += 1;
                } else if current == '.' && !seen_dot {
                    seen_dot = true;
                    index += 1;
                } else if current == ',' && is_thousands_separator(&chars, index) {
                    index += 1;
                } else {
                    break;
                }
            }
            let raw: String = chars[start..index]
                .iter()
                .filter(|c| **c != '_' && **c != ',')
                .collect();
            let number = raw.parse::<f64>().ok()?;
            if !number.is_finite() {
                return None;
            }
            tokens.push(Token::Number(number));
            continue;
        }

        if ch.is_alphabetic() || ch == 'π' {
            let start = index;
            index += 1;
            while index < chars.len() && (chars[index].is_alphabetic() || chars[index] == 'π') {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            let lower = word.to_lowercase();
            match lower.as_str() {
                "of" => tokens.push(Token::Star),
                "mod" => tokens.push(Token::Mod),
                "x" => tokens.push(Token::Star),
                _ => tokens.push(Token::Ident(lower)),
            }
            continue;
        }

        match ch {
            '+' => tokens.push(Token::Plus),
            '-' | '−' => tokens.push(Token::Minus),
            '*' | '×' => tokens.push(Token::Star),
            '/' | '÷' => tokens.push(Token::Slash),
            '^' => tokens.push(Token::Caret),
            '%' => tokens.push(Token::Percent),
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            ',' => tokens.push(Token::Comma),
            _ => return None,
        }
        index += 1;
    }

    Some(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn parse_expression(&mut self) -> Option<Expr> {
        let mut left = self.parse_term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.position += 1;
                    let right = self.parse_term()?;
                    left = Expr::Binary(BinaryOp::Add, Box::new(left), Box::new(right));
                }
                Some(Token::Minus) => {
                    self.position += 1;
                    let right = self.parse_term()?;
                    left = Expr::Binary(BinaryOp::Sub, Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Some(left)
    }

    fn parse_term(&mut self) -> Option<Expr> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinaryOp::Mul,
                Some(Token::Slash) => BinaryOp::Div,
                Some(Token::Mod) => BinaryOp::Mod,
                _ => break,
            };
            self.position += 1;
            let right = self.parse_unary()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        match self.peek() {
            Some(Token::Minus) => {
                self.position += 1;
                Some(Expr::Negate(Box::new(self.parse_unary()?)))
            }
            Some(Token::Plus) => {
                self.position += 1;
                self.parse_unary()
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Option<Expr> {
        let base = self.parse_postfix()?;
        if matches!(self.peek(), Some(Token::Caret)) {
            self.position += 1;
            let exponent = self.parse_unary()?;
            return Some(Expr::Binary(
                BinaryOp::Pow,
                Box::new(base),
                Box::new(exponent),
            ));
        }
        Some(base)
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary()?;
        while matches!(self.peek(), Some(Token::Percent)) {
            self.position += 1;
            expr = Expr::Percent(Box::new(expr));
        }
        Some(expr)
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        match self.next()? {
            Token::Number(number) => Some(Expr::Number(number)),
            Token::Ident(name) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.position += 1;
                    let mut args = Vec::new();
                    if matches!(self.peek(), Some(Token::RParen)) {
                        self.position += 1;
                    } else {
                        loop {
                            args.push(self.parse_expression()?);
                            match self.next()? {
                                Token::Comma => continue,
                                Token::RParen => break,
                                _ => return None,
                            }
                        }
                    }
                    Some(Expr::Call(name, args))
                } else {
                    match name.as_str() {
                        "pi" | "π" => Some(Expr::Number(std::f64::consts::PI)),
                        "e" => Some(Expr::Number(std::f64::consts::E)),
                        _ => None,
                    }
                }
            }
            Token::LParen => {
                let inner = self.parse_expression()?;
                if !matches!(self.next()?, Token::RParen) {
                    return None;
                }
                Some(inner)
            }
            _ => None,
        }
    }
}

fn evaluate(expr: &Expr) -> Option<f64> {
    let value = match expr {
        Expr::Number(number) => *number,
        Expr::Percent(inner) => evaluate(inner)? / 100.0,
        Expr::Negate(inner) => -evaluate(inner)?,
        Expr::Binary(op, left, right) => {
            let left = evaluate(left)?;
            match op {
                BinaryOp::Add => {
                    let right = evaluate_percent_aware(right, left)?;
                    left + right
                }
                BinaryOp::Sub => {
                    let right = evaluate_percent_aware(right, left)?;
                    left - right
                }
                BinaryOp::Mul => left * evaluate(right)?,
                BinaryOp::Div => {
                    let right = evaluate(right)?;
                    if right == 0.0 {
                        return None;
                    }
                    left / right
                }
                BinaryOp::Mod => {
                    let right = evaluate(right)?;
                    if right == 0.0 {
                        return None;
                    }
                    left % right
                }
                BinaryOp::Pow => {
                    let right = evaluate(right)?;
                    left.powf(right)
                }
            }
        }
        Expr::Call(name, args) => evaluate_call(name, args)?,
    };

    value.is_finite().then_some(value)
}

fn evaluate_percent_aware(expr: &Expr, base: f64) -> Option<f64> {
    match expr {
        Expr::Percent(inner) => Some(base * evaluate(inner)? / 100.0),
        Expr::Negate(inner) => evaluate_percent_aware(inner, base).map(|value| -value),
        _ => evaluate(expr),
    }
}

fn evaluate_call(name: &str, args: &[Expr]) -> Option<f64> {
    let mut values = Vec::with_capacity(args.len());
    for arg in args {
        values.push(evaluate(arg)?);
    }

    let unary = |value: f64, f: fn(f64) -> f64| f(value);
    match (name, values.as_slice()) {
        ("sqrt", [value]) if *value >= 0.0 => Some(value.sqrt()),
        ("abs", [value]) => Some(value.abs()),
        ("round", [value]) => Some(value.round()),
        ("floor", [value]) => Some(value.floor()),
        ("ceil", [value]) => Some(value.ceil()),
        ("sin", [value]) => Some(unary(*value, f64::sin)),
        ("cos", [value]) => Some(unary(*value, f64::cos)),
        ("tan", [value]) => Some(unary(*value, f64::tan)),
        ("ln", [value]) if *value > 0.0 => Some(value.ln()),
        ("log", [value]) if *value > 0.0 => Some(value.log10()),
        ("exp", [value]) => Some(value.exp()),
        ("min", [left, right]) => Some(left.min(*right)),
        ("max", [left, right]) => Some(left.max(*right)),
        ("pow", [left, right]) => Some(left.powf(*right)),
        _ => None,
    }
}

fn is_constant(name: &str) -> bool {
    matches!(name, "pi" | "π" | "e")
}

fn is_function(name: &str) -> bool {
    matches!(
        name,
        "sqrt"
            | "abs"
            | "round"
            | "floor"
            | "ceil"
            | "sin"
            | "cos"
            | "tan"
            | "ln"
            | "log"
            | "exp"
            | "min"
            | "max"
            | "pow"
    )
}

fn format_plain(value: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if value.fract().abs() < 1e-10 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }

    let mut text = format!("{value:.10}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn group_thousands(text: &str) -> String {
    let (sign, body) = match text.strip_prefix('-') {
        Some(body) => ("-", body),
        None => ("", text),
    };
    let (integer, fraction) = match body.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (body, None),
    };

    let mut grouped = String::with_capacity(body.len() + body.len() / 3 + 1);
    for (index, ch) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }

    let mut output = String::with_capacity(grouped.len() + fraction.map_or(0, |f| f.len() + 1) + 1);
    output.push_str(sign);
    output.push_str(&grouped);
    if let Some(fraction) = fraction {
        output.push('.');
        output.push_str(fraction);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(query: &str) -> f64 {
        try_evaluate(query).expect(query).value
    }

    #[test]
    fn basic_arithmetic() {
        assert_eq!(value("2+2"), 4.0);
        assert_eq!(value("2 + 3 * 4"), 14.0);
        assert_eq!(value("(2 + 3) * 4"), 20.0);
        assert_eq!(value("10 / 4"), 2.5);
        assert_eq!(value("2^10"), 1024.0);
        assert_eq!(value("-2^2"), -4.0);
        assert_eq!(value("2^-2"), 0.25);
        assert_eq!(value("10 mod 3"), 1.0);
    }

    #[test]
    fn separators_and_decimals() {
        assert_eq!(value("1,234.5 + 0.5"), 1235.0);
        assert_eq!(value("1_000 + 2"), 1002.0);
        assert_eq!(value(".5 + .25"), 0.75);
    }

    #[test]
    fn percentages() {
        assert_eq!(value("50%"), 0.5);
        assert_eq!(value("50% of 200"), 100.0);
        assert_eq!(value("10% of 250"), 25.0);
        assert_eq!(value("200 + 10%"), 220.0);
        assert_eq!(value("200 - 10%"), 180.0);
        assert_eq!(value("100 + 10% + 10%"), 121.0);
        assert_eq!(value("200 * 10%"), 20.0);
    }

    #[test]
    fn functions_and_constants() {
        assert!((value("sqrt(81)") - 9.0).abs() < 1e-9);
        assert!((value("pi") - std::f64::consts::PI).abs() < 1e-9);
        assert!((value("cos(0)") - 1.0).abs() < 1e-9);
        assert_eq!(value("max(2, 9)"), 9.0);
        assert_eq!(value("min(2, 9)"), 2.0);
    }

    #[test]
    fn formatting() {
        let result = try_evaluate("1234.5+0.5").unwrap();
        assert_eq!(result.copy, "1235");
        assert_eq!(result.display, "1,235");

        let result = try_evaluate("1000000 + 1").unwrap();
        assert_eq!(result.display, "1,000,001");

        let result = try_evaluate("1/3").unwrap();
        assert_eq!(result.copy, "0.3333333333");
    }

    #[test]
    fn rejects_non_math() {
        for query in ["", "abc", "7zip", "2+", "1//2", "chrome", "..", "2 2"] {
            assert!(try_evaluate(query).is_none(), "{query}");
        }
    }

    #[test]
    fn rejects_bad_domains() {
        assert!(try_evaluate("1/0").is_none());
        assert!(try_evaluate("sqrt(-1)").is_none());
        assert!(try_evaluate("ln(0)").is_none());
        assert!(try_evaluate("2^1000000").is_none());
    }

    #[test]
    fn is_expression_requires_operator() {
        assert!(is_expression("2+2"));
        assert!(is_expression("50% of 200"));
        assert!(!is_expression("42"));
    }
}
