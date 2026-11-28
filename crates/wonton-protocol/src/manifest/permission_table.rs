//! 权限表的行类型和生成宏（ADR-0128）。表本身在 [`super::permission`]，每种权限只在那里写一行：
//! 枚举、种类、清单写法的解析和显示、给 App 的说明键都从这一行生成，核心按行查风险、可选和注入变量。

/// 权限的风险级别（ADR-0036）。普通权限没有级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    High,
    VeryHigh,
    Highest,
}

/// 表里的风险一栏。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskRule {
    /// 普通权限，不单独提示。
    Normal,
    High,
    VeryHigh,
    Highest,
    /// 按 Linux capability 的名字定，名单外的按最高级（[`super::capability_risk`]）。
    ByCapability,
}

/// 表里的风险提示一栏：确认页在说明下面多写的一句。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Silent,
    /// 核心 Fluent 目录里的消息键，参数同说明文字。
    Text(&'static str),
    /// 实际级别是最高级时写 [`HIGHEST_RISK_NOTICE`]，否则不写。
    HighestRisk,
}

/// 最高级权限共用的提示。
pub const HIGHEST_RISK_NOTICE: &str = "permission-notice-highest-risk";

/// 权限表的一行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionSpec {
    pub kind: super::PermissionKind,
    /// 清单里的写法。带参数的是模板，占位符就是参数名，如 `storage.share.{name}:{access}`。
    pub syntax: &'static str,
    /// 一个合法的写法；不带参数的就是 `syntax`。
    pub example: &'static str,
    /// 参数名，和 `syntax` 的占位符、说明文字的 Fluent 变量一致；不带参数的为空。
    pub params: &'static [&'static str],
    pub risk: RiskRule,
    /// 能声明成可选、默认关（ADR-0057）。判据：关着时整段字段能拿掉，插件还能运行。
    pub optional: bool,
    /// 可选项开着时注入给插件所有服务的变量名，值固定 `1`，关着时不定义（ADR-0057）。
    pub env: Option<&'static str>,
    /// 给主人看的说明：核心 Fluent 目录里的消息键。
    pub text: &'static str,
    pub notice: Notice,
    /// 核心能兑现。规范里定义了、核心还不能兑现的写 `false`，安装时如实拒绝。
    pub supported: bool,
}

/// 从权限表生成 `Permission`、`PermissionKind`、`PERMISSIONS`，以及解析、显示、种类和参数。
///
/// 带参数的行写 `parse: 前缀 => 函数`：函数拿到去掉前缀的剩余部分，解析不了返回 `None`。不带参数的行按
/// `syntax` 全文匹配，先于前缀尝试。显示用 `syntax` 当格式串，占位符和字段对不上时编译不过。
macro_rules! permission_table {
    (@example $syntax:literal) => {
        $syntax
    };
    (@example $syntax:literal, $example:literal) => {
        $example
    };
    (@exact $value:ident, $syntax:literal, $variant:ident) => {
        if $value == $syntax {
            return Some(Self::$variant);
        }
    };
    (@exact $value:ident, $syntax:literal, $variant:ident { $($field:ident)+ }) => {};
    (
        $(
            $(#[$meta:meta])*
            $variant:ident $({ $($field:ident: $ty:ty),+ $(,)? })? = $syntax:literal {
                $(parse: $prefix:literal => $parse:path, example: $example:literal,)?
                risk: $risk:ident,
                optional: $optional:literal,
                env: $env:expr,
                text: $text:literal,
                notice: $notice:ident $(($notice_key:literal))?,
                supported: $supported:literal $(,)?
            }
        ),+ $(,)?
    ) => {
        /// 清单里声明、安装时由主人确认的权限。每种的写法和属性见 [`PERMISSIONS`]。
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub enum Permission {
            $( $(#[$meta])* $variant $({ $($field: $ty),+ })?, )+
        }

        /// 权限的种类，不带参数。
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum PermissionKind {
            $( $variant, )+
        }

        /// 权限表，每种权限一行，顺序同 [`PermissionKind`]。
        pub const PERMISSIONS: &[PermissionSpec] = &[
            $(
                PermissionSpec {
                    kind: PermissionKind::$variant,
                    syntax: $syntax,
                    example: permission_table!(@example $syntax $(, $example)?),
                    params: &[$($(stringify!($field)),+)?],
                    risk: RiskRule::$risk,
                    optional: $optional,
                    env: $env,
                    text: $text,
                    notice: Notice::$notice $(($notice_key))?,
                    supported: $supported,
                },
            )+
        ];

        impl Permission {
            pub fn kind(&self) -> PermissionKind {
                match self {
                    $( Self::$variant $({ $($field: _),+ })? => PermissionKind::$variant, )+
                }
            }

            /// 参数名和值，供说明文字和提示当 Fluent 变量用。
            pub fn params(&self) -> Vec<(&'static str, String)> {
                match self {
                    $(
                        Self::$variant $({ $($field),+ })? => {
                            vec![$($((stringify!($field), $field.to_string())),+)?]
                        }
                    )+
                }
            }

            fn parse(value: &str) -> Option<Self> {
                $( permission_table!(@exact value, $syntax, $variant $({ $($field)+ })?); )+
                $(
                    $(
                        if let Some(rest) = value.strip_prefix($prefix) {
                            return $parse(rest);
                        }
                    )?
                )+
                None
            }
        }

        impl fmt::Display for Permission {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(
                        Self::$variant $({ $($field),+ })? => {
                            write!(f, $syntax $($(, $field = $field)+)?)
                        }
                    )+
                }
            }
        }
    };
}

pub(super) use permission_table;
