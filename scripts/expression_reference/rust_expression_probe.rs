// The verification script substitutes a JSON-escaped absolute module path.
#[path = __EXPRESSION_MODULE_PATH__]
mod expression;

fn main() {
    for text in std::env::args().skip(1) {
        println!("{}\t{:?}", text, expression::evaluate(&text));
    }
}
