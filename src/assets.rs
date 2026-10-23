pub struct Asset {
    pub name: &'static str,
    pub sha256: &'static str,
    pub loader: &'static str,
    pub loader_sha256: &'static str,
    pub preloader_sha256: &'static str,
    pub core_sha256: &'static str,
    pub release: &'static str,
    pub coreclr: Option<&'static str>,
}

const MONO_RELEASE: &str = "v5.4.23.5";
#[cfg(not(target_os = "macos"))]
const MONO_CORE_SHA256: &str = "8255b28902886085c578b9e427d3073c97002db85176d2090cdeda90ef14ce70";
#[cfg(not(target_os = "macos"))]
const MONO_PRELOADER_SHA256: &str =
    "55d3895351a9d16b63b6f35f1c01b44ac650979e853d0bd3a442b92a082af64f";
const IL2CPP_RELEASE: &str = "v6.0.0-pre.2";
const IL2CPP_CORE_SHA256: &str = "25098c1d5d71341c0dc07184a06ce3a43f8c2398249741e31cdd7946d1ab27f2";
const IL2CPP_PRELOADER_SHA256: &str =
    "d77b1d354f74db1602ed4dc10b4f0ec204387e483672cd5b66a65eda223930d1";

#[cfg(target_os = "macos")]
pub const MONO_MACOS: Asset = Asset {
    name: "BepInEx_macos_universal_5.4.23.5.zip",
    sha256: "01c2ae782eb016dfd6c345a18dbd2dcafffb3d9d318449d6486689f426b4a323",
    loader: "libdoorstop.dylib",
    loader_sha256: "cb4aaa97bd9a08178ac2d165b33284b744d18498d5dec5a07fc2b6f6d87d80b9",
    preloader_sha256: "309dd5f1f1dda9209dfc4522a29ac983994f0cca135dee7012582028e9a47627",
    core_sha256: "19eb836818955e4f86818306aaf2baaee989be566d7da697f835b429ab585149",
    release: MONO_RELEASE,
    coreclr: None,
};

#[cfg(target_os = "linux")]
pub const MONO_LINUX_X64: Asset = Asset {
    name: "BepInEx_linux_x64_5.4.23.5.zip",
    sha256: "e538560be65739f562519ab518a75f9c65b3f57f87457403ae7cde683c12dab7",
    loader: "libdoorstop.so",
    loader_sha256: "07ec6ee28c7d200c000ba9db6dfecec466a6ed449194a37decb719bf5321aee0",
    preloader_sha256: MONO_PRELOADER_SHA256,
    core_sha256: MONO_CORE_SHA256,
    release: MONO_RELEASE,
    coreclr: None,
};

#[cfg(target_os = "linux")]
pub const MONO_LINUX_X86: Asset = Asset {
    name: "BepInEx_linux_x86_5.4.23.5.zip",
    sha256: "b4a815b33635921faedeada0df00ba5436b4441d9e16bb13f4bc8c7ad4edde63",
    loader: "libdoorstop.so",
    loader_sha256: "edb4d13fd5cd1949d7ec8919cea314121b8fa7d0243d853ced3a9e70b9156674",
    preloader_sha256: MONO_PRELOADER_SHA256,
    core_sha256: MONO_CORE_SHA256,
    release: MONO_RELEASE,
    coreclr: None,
};

#[cfg(target_os = "windows")]
pub const MONO_WIN_X64: Asset = Asset {
    name: "BepInEx_win_x64_5.4.23.5.zip",
    sha256: "82f9878551030f54657792c0740d9d51a09500eeae1fba21106b0c441e6732c4",
    loader: "winhttp.dll",
    loader_sha256: "8c6cdbc38836dee87e3368f5de1994d7c0ccebf29e4ce7aba3c0981f9375412c",
    preloader_sha256: MONO_PRELOADER_SHA256,
    core_sha256: MONO_CORE_SHA256,
    release: MONO_RELEASE,
    coreclr: None,
};

#[cfg(target_os = "windows")]
pub const MONO_WIN_X86: Asset = Asset {
    name: "BepInEx_win_x86_5.4.23.5.zip",
    sha256: "37651c79e40d6f909572a4f461ac25350bb3ef8fe7fbd29f1aa8791a33b84c82",
    loader: "winhttp.dll",
    loader_sha256: "cc643b54484f694a8e0e6641cac79d74141009afc9e24d826f6fd7fd48fd182a",
    preloader_sha256: MONO_PRELOADER_SHA256,
    core_sha256: MONO_CORE_SHA256,
    release: MONO_RELEASE,
    coreclr: None,
};

#[cfg(target_os = "macos")]
pub const IL2CPP_MACOS_X64: Asset = Asset {
    name: "BepInEx-Unity.IL2CPP-macos-x64-6.0.0-pre.2.zip",
    sha256: "9f6a849e8a5912a575f460232649df42de7793489df1531e64bd7ab24dc4b893",
    loader: "libdoorstop.dylib",
    loader_sha256: "c1973e0b124b7ca58aed82c6672559f411da214760718eb42b028bd3f8f57e16",
    preloader_sha256: IL2CPP_PRELOADER_SHA256,
    core_sha256: IL2CPP_CORE_SHA256,
    release: IL2CPP_RELEASE,
    coreclr: Some("dotnet/libcoreclr.dylib"),
};

#[cfg(target_os = "linux")]
pub const IL2CPP_LINUX_X64: Asset = Asset {
    name: "BepInEx-Unity.IL2CPP-linux-x64-6.0.0-pre.2.zip",
    sha256: "e63c34e49d080b19f14ece4af799d3d8c27c32cac9c81903af99dd9eea414a0e",
    loader: "libdoorstop.so",
    loader_sha256: "f29dee8d59729b0899924f870346a9524df3bcefe11aa9d0f009d523f6b33cce",
    preloader_sha256: IL2CPP_PRELOADER_SHA256,
    core_sha256: IL2CPP_CORE_SHA256,
    release: IL2CPP_RELEASE,
    coreclr: Some("dotnet/libcoreclr.so"),
};

#[cfg(target_os = "windows")]
pub const IL2CPP_WIN_X64: Asset = Asset {
    name: "BepInEx-Unity.IL2CPP-win-x64-6.0.0-pre.2.zip",
    sha256: "616ec7eb06cf11b2a0000e8fcef04d1b12bb58e84a2e0bdac9523234fc193ceb",
    loader: "winhttp.dll",
    loader_sha256: "d3b62f4c9c3e2196ef82603c52d6b98c043a0d6c125c081fd33d7ca3798b41b8",
    preloader_sha256: IL2CPP_PRELOADER_SHA256,
    core_sha256: IL2CPP_CORE_SHA256,
    release: IL2CPP_RELEASE,
    coreclr: Some("dotnet/coreclr.dll"),
};

#[cfg(target_os = "windows")]
pub const IL2CPP_WIN_X86: Asset = Asset {
    name: "BepInEx-Unity.IL2CPP-win-x86-6.0.0-pre.2.zip",
    sha256: "cfef3a1e946dac5db8b9de4de1a922f47584dd775da32863f36762fbaad80f19",
    loader: "winhttp.dll",
    loader_sha256: "80a5988815fccf70fba37b9baa4fc5e39d869969f7ff9ec77a9efbdcfed10183",
    preloader_sha256: IL2CPP_PRELOADER_SHA256,
    core_sha256: IL2CPP_CORE_SHA256,
    release: IL2CPP_RELEASE,
    coreclr: Some("dotnet/coreclr.dll"),
};
