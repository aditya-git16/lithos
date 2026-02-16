use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct BinanceDto<'a> {
    pub u: u64,     // order book updateId
    pub s: &'a str, // symbol
    pub b: &'a str, // best bid price
    #[serde(rename = "B")]
    pub b_qty: &'a str, // best bid qty
    pub a: &'a str, // best ask price
    #[serde(rename = "A")]
    pub a_qty: &'a str, // best ask qty
}
