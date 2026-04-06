//! Registry of well-known Ethereum mainnet contract addresses and method selectors.
//!
//! Used by the block observer to generate semantically-rich insights about
//! real DeFi activity rather than just aggregate gas stats.

/// Category of contract for grouping insights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum ContractCategory {
    DexRouter,
    LendingPool,
    Stablecoin,
    Weth,
    Lst,        // liquid staking token
    NftMarket,
    Bridge,
    Mev,
    Restaking,
    Other,
}

impl ContractCategory {
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            Self::DexRouter => "DEX",
            Self::LendingPool => "lending",
            Self::Stablecoin => "stablecoin",
            Self::Weth => "WETH",
            Self::Lst => "LST",
            Self::NftMarket => "NFT",
            Self::Bridge => "bridge",
            Self::Mev => "MEV",
            Self::Restaking => "restaking",
            Self::Other => "contract",
        }
    }
}

/// A labeled contract entry.
#[derive(Clone, Copy, Debug)]
pub struct KnownContract {
    pub name: &'static str,
    pub category: ContractCategory,
}

/// Lookup a known address. Case-insensitive (normalizes to lowercase).
pub fn lookup(addr: &str) -> Option<KnownContract> {
    let a = addr.trim().to_lowercase();
    let key = if let Some(s) = a.strip_prefix("0x") { s } else { &a };
    KNOWN_ADDRESSES
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| *v)
}

/// Decode a transaction's input data first 4 bytes → method name.
pub fn decode_method_selector(input: &str) -> Option<&'static str> {
    let s = input.strip_prefix("0x").unwrap_or(input);
    if s.len() < 8 {
        return None;
    }
    let selector = &s[..8].to_lowercase();
    METHOD_SELECTORS
        .iter()
        .find(|(k, _)| *k == selector)
        .map(|(_, v)| *v)
}

/// `(address_hex_no_prefix, (name, category))` pairs — all lowercased.
const KNOWN_ADDRESSES: &[(&str, KnownContract)] = &[
    // Uniswap
    ("7a250d5630b4cf539739df2c5dacb4c659f2488d", KnownContract{ name: "Uniswap V2 Router", category: ContractCategory::DexRouter }),
    ("e592427a0aece92de3edee1f18e0157c05861564", KnownContract{ name: "Uniswap V3 Router", category: ContractCategory::DexRouter }),
    ("68b3465833fb72a70ecdf485e0e4c7bd8665fc45", KnownContract{ name: "Uniswap V3 Router 2", category: ContractCategory::DexRouter }),
    ("ef1c6e67703c7bd7107eed8303fbe6ec2554bf6b", KnownContract{ name: "Uniswap Universal Router", category: ContractCategory::DexRouter }),
    ("3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad", KnownContract{ name: "Uniswap Universal Router v1.2", category: ContractCategory::DexRouter }),
    ("66a9893cc07d91d95644aedd05d03f95e1dba8af", KnownContract{ name: "Uniswap V4 Router", category: ContractCategory::DexRouter }),
    // SushiSwap
    ("d9e1ce17f2641f24ae83637ab66a2cca9c378b9f", KnownContract{ name: "SushiSwap V2 Router", category: ContractCategory::DexRouter }),
    // Curve
    ("99a58482bd75cbab83b27ec03ca68ff489b5788f", KnownContract{ name: "Curve Router", category: ContractCategory::DexRouter }),
    ("fa9a30350048b2bf66865ee20363067c66f67e58", KnownContract{ name: "Curve Router v1.2", category: ContractCategory::DexRouter }),
    // 1inch
    ("1111111254eeb25477b68fb85ed929f73a960582", KnownContract{ name: "1inch Router v5", category: ContractCategory::DexRouter }),
    ("111111125421ca6dc452d289314280a0f8842a65", KnownContract{ name: "1inch Router v6", category: ContractCategory::DexRouter }),
    // 0x Exchange
    ("def1c0ded9bec7f1a1670819833240f027b25eff", KnownContract{ name: "0x Exchange Proxy", category: ContractCategory::DexRouter }),
    // Paraswap
    ("6a000f20005980200259b80c5102003040001068", KnownContract{ name: "ParaSwap V6", category: ContractCategory::DexRouter }),
    // Aave
    ("87870bca3f3fd6335c3f4ce8392d69350b4fa4e2", KnownContract{ name: "Aave V3 Pool", category: ContractCategory::LendingPool }),
    ("7d2768de32b0b80b7a3454c06bdac94a69ddc7a9", KnownContract{ name: "Aave V2 LendingPool", category: ContractCategory::LendingPool }),
    // Compound
    ("4ddc2d193948926d02f9b1fe9e1daa0718270ed5", KnownContract{ name: "Compound cETH", category: ContractCategory::LendingPool }),
    ("c3d688b66703497daa19211eedff47f25384cdc3", KnownContract{ name: "Compound Comet USDC", category: ContractCategory::LendingPool }),
    // Morpho
    ("bbbbbbbbbb9cc5e90e3b3af64bdaf62c37eeffcb", KnownContract{ name: "Morpho Blue", category: ContractCategory::LendingPool }),
    // Stablecoins (ERC-20 contracts)
    ("a0b86991c6218b36c1d19d4a2e9eb0ce3606eb48", KnownContract{ name: "USDC", category: ContractCategory::Stablecoin }),
    ("dac17f958d2ee523a2206206994597c13d831ec7", KnownContract{ name: "USDT", category: ContractCategory::Stablecoin }),
    ("6b175474e89094c44da98b954eedeac495271d0f", KnownContract{ name: "DAI", category: ContractCategory::Stablecoin }),
    ("853d955acef822db058eb8505911ed77f175b99e", KnownContract{ name: "FRAX", category: ContractCategory::Stablecoin }),
    ("4c9edd5852cd905f086c759e8383e09bff1e68b3", KnownContract{ name: "USDe", category: ContractCategory::Stablecoin }),
    ("8292bb45bf1ee4d140127049757c2e0ff06317ed", KnownContract{ name: "RLUSD", category: ContractCategory::Stablecoin }),
    // WETH
    ("c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2", KnownContract{ name: "WETH", category: ContractCategory::Weth }),
    // Liquid staking
    ("ae7ab96520de3a18e5e111b5eaab095312d7fe84", KnownContract{ name: "Lido stETH", category: ContractCategory::Lst }),
    ("7f39c581f595b53c5cb19bd0b3f8da6c935e2ca0", KnownContract{ name: "Lido wstETH", category: ContractCategory::Lst }),
    ("a35b1b31ce002fbf2058d22f30f95d405200a15b", KnownContract{ name: "ETHx", category: ContractCategory::Lst }),
    ("ae78736cd615f374d3085123a210448e74fc6393", KnownContract{ name: "rETH (Rocket)", category: ContractCategory::Lst }),
    // Restaking
    ("858646372cc42e1a627fce94aa7a7033e7cf075a", KnownContract{ name: "EigenLayer Strategy Manager", category: ContractCategory::Restaking }),
    ("a1290d69c65a6fe4df752f95823fae25cb99e5a7", KnownContract{ name: "Renzo ezETH", category: ContractCategory::Restaking }),
    ("e3cbd06d7dadb3f4e6557bab7edd924cd1489e8f", KnownContract{ name: "Kelp rsETH", category: ContractCategory::Restaking }),
    // NFT markets
    ("00000000000000adc04c56bf30ac9d3c0aaf14dc", KnownContract{ name: "Seaport 1.5 (OpenSea)", category: ContractCategory::NftMarket }),
    ("0000000000000068f116a894984e2db1123eb395", KnownContract{ name: "Seaport 1.6", category: ContractCategory::NftMarket }),
    ("000000000000ad05ccc4f10045630fb830b95127", KnownContract{ name: "Blur Marketplace", category: ContractCategory::NftMarket }),
    ("29469395eaf6f95920e59f858042f0e28d98a20b", KnownContract{ name: "Blur Pool", category: ContractCategory::NftMarket }),
    // Bridges
    ("8315177ab297ba92a06054ce80a67ed4dbd7ed3a", KnownContract{ name: "Arbitrum Bridge", category: ContractCategory::Bridge }),
    ("25ace71c97b33cc4729cf772ae268934f7ab5fa1", KnownContract{ name: "Optimism Bridge (L1)", category: ContractCategory::Bridge }),
    ("a0c68c638235ee32657e8f720a23cec1bfc77c77", KnownContract{ name: "Polygon PoS Bridge", category: ContractCategory::Bridge }),
    ("49048044d57e1c92a77f79988d21fa8faf74e97e", KnownContract{ name: "Base Bridge", category: ContractCategory::Bridge }),
    // MEV builders / searchers (known addresses)
    ("95222290dd7278aa3ddd389cc1e1d165cc4bafe5", KnownContract{ name: "beaverbuild (builder)", category: ContractCategory::Mev }),
    ("1f9090aae28b8a3dceadf281b0f12828e676c326", KnownContract{ name: "rsync-builder", category: ContractCategory::Mev }),
    ("dafea492d9c6733ae3d56b7ed1adb60692c98bc5", KnownContract{ name: "Flashbots Builder", category: ContractCategory::Mev }),
    ("b646d87963da1fb9d192ddba775f24f33e857128", KnownContract{ name: "Titan Builder", category: ContractCategory::Mev }),
];

/// `(4-byte hex, method_name)` pairs. Lowercase hex, no `0x`.
const METHOD_SELECTORS: &[(&str, &str)] = &[
    // ERC-20
    ("a9059cbb", "transfer"),
    ("23b872dd", "transferFrom"),
    ("095ea7b3", "approve"),
    ("40c10f19", "mint"),
    ("42966c68", "burn"),
    // Permit2
    ("2b67b570", "permit"),
    ("36c78516", "permitTransferFrom"),
    // Uniswap V2
    ("7ff36ab5", "swapExactETHForTokens"),
    ("38ed1739", "swapExactTokensForTokens"),
    ("18cbafe5", "swapExactTokensForETH"),
    ("fb3bdb41", "swapETHForExactTokens"),
    // Uniswap V3
    ("414bf389", "exactInputSingle"),
    ("c04b8d59", "exactInput"),
    ("db3e2198", "exactOutputSingle"),
    ("09b81346", "exactOutput"),
    ("5ae401dc", "multicall"),
    // Uniswap Universal Router
    ("3593564c", "execute"),
    ("24856bc3", "execute (v1.2)"),
    // Aave V3
    ("617ba037", "supply (Aave)"),
    ("69328dec", "withdraw (Aave)"),
    ("a415bcad", "borrow (Aave)"),
    ("573ade81", "repay (Aave)"),
    // Lido
    ("a1903eab", "submit (Lido)"),
    ("1a9a26f3", "requestWithdrawal"),
    // Compound
    ("1249c58b", "mint (Compound)"),
    ("852a12e3", "redeem"),
    ("a6afed95", "accrueInterest"),
    // 0x
    ("d9627aa4", "sellToUniswap (0x)"),
    ("415565b0", "transformERC20 (0x)"),
    // 1inch
    ("12aa3caf", "swap (1inch)"),
    ("e449022e", "uniswapV3Swap (1inch)"),
    ("84bd6d29", "clipperSwap (1inch)"),
    // Flashbots-style
    ("5cf54026", "flashbotsCheckAndSend"),
    // Seaport
    ("fb0f3ee1", "fulfillBasicOrder"),
    ("fd9f1e10", "fulfillAvailableAdvancedOrders"),
    ("a8174404", "fulfillOrder"),
    ("e7acab24", "fulfillAdvancedOrder"),
    // Blur
    ("9a1fc3a7", "bulkExecute"),
    ("70bce2d6", "execute (Blur)"),
    // Multicall
    ("252dba42", "aggregate (Multicall)"),
    ("82ad56cb", "aggregate3 (Multicall3)"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_usdc() {
        let c = lookup("0xa0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").unwrap();
        assert_eq!(c.name, "USDC");
        assert_eq!(c.category, ContractCategory::Stablecoin);
    }

    #[test]
    fn lookup_case_insensitive() {
        assert!(lookup("0xA0B86991C6218B36C1D19D4A2E9EB0CE3606EB48").is_some());
        assert!(lookup("A0B86991C6218B36C1D19D4A2E9EB0CE3606EB48").is_some());
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("0x0000000000000000000000000000000000000000").is_none());
    }

    #[test]
    fn decode_transfer_selector() {
        assert_eq!(decode_method_selector("0xa9059cbb000000"), Some("transfer"));
        assert_eq!(decode_method_selector("a9059cbb000000"), Some("transfer"));
    }

    #[test]
    fn decode_unknown_selector() {
        assert_eq!(decode_method_selector("0xdeadbeef000000"), None);
        assert_eq!(decode_method_selector("0x"), None);
    }

    #[test]
    fn category_labels_stable() {
        assert_eq!(ContractCategory::DexRouter.label(), "DEX");
        assert_eq!(ContractCategory::LendingPool.label(), "lending");
    }
}
