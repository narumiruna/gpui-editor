use std::sync::LazyLock;

use regress::Regex;

use super::{SimpleRule, next, styled};

pub(super) fn states() -> [(&'static str, Vec<SimpleRule>); 2] {
    [
        (
            "start",
            vec![
                // Numbers
                styled(
                    r"(?:[+-]?)(?:0x[\d,a-f]+)|(?:0o[0-7]+)|(?:0b[0,1]+)|(?:\d+.?\d*)",
                    "number",
                ),
                // Strings
                styled(r#""(?:[^\\"]|\\.)*"?"#, "string"),
                styled(r"'(?:[^\\']|\\.)*'?", "string"),
                styled(r"`(?:[^\\`]|\\.)*`?", "string"),
                // Compile Time Commands
                SimpleRule {
                    ignore_case: true,
                    ..styled(
                        concat!(
                            r"^\s*(?:\!(addincludedir|addplugindir|appendfile|appendmemfile|assert|cd|define|delfile|echo|error|",
                            r"execute|finalize|getdllversion|gettlbversion|include|insertmacro|macro|macroend|macroundef|",
                            r"makensis|packhdr|pragma|searchparse|searchreplace|system|tempfile|undef|uninstfinalize|verbose|",
                            r"warning))\b",
                        ),
                        "keyword",
                    )
                },
                // Conditional Compilation
                SimpleRule {
                    ignore_case: true,
                    indent: true,
                    ..styled(r"^\s*(?:\!(if(?:n?def)?|ifmacron?def|macro))\b", "keyword")
                },
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(r"^\s*(?:\!(else|endif|macroend))\b", "keyword")
                },
                // Runtime Commands
                SimpleRule {
                    ignore_case: true,
                    ..styled(
                        concat!(
                            r"^\s*(?:Abort|AddBrandingImage|AddSize|AllowRootDirInstall|AllowSkipFiles|AutoCloseWindow|BGFont|",
                            r"BGGradient|BrandingText|BringToFront|Call|CallInstDLL|Caption|ChangeUI|CheckBitmap|ClearErrors|",
                            r"CompletedText|ComponentText|CopyFiles|CRCCheck|CreateDirectory|CreateFont|CreateShortCut|Delete|",
                            r"DeleteINISec|DeleteINIStr|DeleteRegKey|DeleteRegValue|DetailPrint|DetailsButtonText|DirText|DirVar|",
                            r"DirVerify|EnableWindow|EnumRegKey|EnumRegValue|Exch|Exec|ExecShell|ExecShellWait|ExecWait|",
                            r"ExpandEnvStrings|File|FileBufSize|FileClose|FileErrorText|FileOpen|FileRead|FileReadByte|",
                            r"FileReadUTF16LE|FileReadWord|FileWriteUTF16LE|FileSeek|FileWrite|FileWriteByte|FileWriteWord|",
                            r"FindClose|FindFirst|FindNext|FindWindow|FlushINI|GetCurInstType|GetCurrentAddress|GetDlgItem|",
                            r"GetDLLVersion|GetDLLVersionLocal|GetErrorLevel|GetFileTime|GetFileTimeLocal|GetFullPathName|",
                            r"GetFunctionAddress|GetInstDirError|GetKnownFolderPath|GetLabelAddress|GetRegView|",
                            r"GetShellVarContext|GetTempFileName|GetWinVer|Goto|HideWindow|Icon|IfAbort|IfAltRegView|IfErrors|",
                            r"IfFileExists|IfRebootFlag|IfRtlLanguage|IfShellVarContextAll|IfSilent|InitPluginsDir|",
                            r"InstallButtonText|InstallColors|InstallDir|InstallDirRegKey|InstProgressFlags|InstType|",
                            r"InstTypeGetText|InstTypeSetText|Int64Cmp|Int64CmpU|Int64Fmt|IntCmp|IntCmpU|IntFmt|IntOp|IntPtrCmp|",
                            r"IntPtrCmpU|IntPtrOp|IsWindow|LangString|LicenseBkColor|LicenseData|LicenseForceSelection|",
                            r"LicenseLangString|LicenseText|LoadAndSetImage|LoadLanguageFile|LockWindow|LogSet|LogText|",
                            r"ManifestAppendCustomString|ManifestDisableWindowFiltering|ManifestDPIAware|ManifestDPIAwareness|",
                            r"ManifestGdiScaling|ManifestLongPathAware|ManifestMaxVersionTested|ManifestSupportedOS|MessageBox|",
                            r"MiscButtonText|Name|Nop|OutFile|Page|PageCallbacks|PEAddResource|PEDllCharacteristics|",
                            r"PERemoveResource|PESubsysVer|Pop|Push|Quit|ReadEnvStr|ReadINIStr|ReadMemory|ReadRegDWORD|",
                            r"ReadRegStr|Reboot|RegDLL|Rename|RequestExecutionLevel|ReserveFile|Return|RMDir|SearchPath|",
                            r"SectionGetFlags|SectionGetInstTypes|SectionGetSize|SectionGetText|SectionIn|SectionInstType|",
                            r"SectionSetFlags|SectionSetInstTypes|SectionSetSize|SectionSetText|SendMessage|SetAutoClose|",
                            r"SetBrandingImage|SetCompress|SetCompressionLevel|SetCompressor|SetCompressorDictSize|SetCtlColors|",
                            r"SetCurInstType|SetDatablockOptimize|SetDateSave|SetDetailsPrint|SetDetailsView|SetErrorLevel|",
                            r"SetErrors|SetFileAttributes|SetFont|SetOutPath|SetOverwrite|SetRebootFlag|SetRegView|",
                            r"SetShellVarContext|SetSilent|ShowInstDetails|ShowUninstDetails|ShowWindow|SilentInstall|",
                            r"SilentUnInstall|Sleep|SpaceTexts|StrCmp|StrCmpS|StrCpy|StrLen|SubCaption|Target|Unicode|",
                            r"UnsafeStrCpy|UninstallButtonText|UninstallCaption|UninstallIcon|UninstallSubCaption|UninstallText|",
                            r"UninstPage|UnRegDLL|Var|VIAddVersionKey|VIFileVersion|VIProductVersion|WindowIcon|WriteINIStr|",
                            r"WriteRegBin|WriteRegDWORD|WriteRegExpandStr|WriteRegMultiStr|WriteRegNone|WriteRegStr|",
                            r"WriteUninstaller|XPStyle)\b",
                        ),
                        "keyword",
                    )
                },
                SimpleRule {
                    ignore_case: true,
                    indent: true,
                    ..styled(r"^\s*(?:Function|PageEx|Section(?:Group)?)\b", "keyword")
                },
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(
                        r"^\s*(?:(Function|PageEx|Section(?:Group)?)End)\b",
                        "keyword",
                    )
                },
                // Command Options
                SimpleRule {
                    ignore_case: true,
                    ..styled(
                        concat!(
                            r"\b(?:ARCHIVE|FILE_ATTRIBUTE_ARCHIVE|FILE_ATTRIBUTE_HIDDEN|FILE_ATTRIBUTE_NORMAL|",
                            r"FILE_ATTRIBUTE_OFFLINE|FILE_ATTRIBUTE_READONLY|FILE_ATTRIBUTE_SYSTEM|FILE_ATTRIBUTE_TEMPORARY|",
                            r"HIDDEN|HKCC|HKCR(32|64)?|HKCU(32|64)?|HKDD|HKEY_CLASSES_ROOT|HKEY_CURRENT_CONFIG|HKEY_CURRENT_USER|",
                            r"HKEY_DYN_DATA|HKEY_LOCAL_MACHINE|HKEY_PERFORMANCE_DATA|HKEY_USERS|HKLM(32|64)?|HKPD|HKU|IDABORT|",
                            r"IDCANCEL|IDD_DIR|IDD_INST|IDD_INSTFILES|IDD_LICENSE|IDD_SELCOM|IDD_UNINST|IDD_VERIFY|IDIGNORE|IDNO|",
                            r"IDOK|IDRETRY|IDYES|MB_ABORTRETRYIGNORE|MB_DEFBUTTON1|MB_DEFBUTTON2|MB_DEFBUTTON3|MB_DEFBUTTON4|",
                            r"MB_ICONEXCLAMATION|MB_ICONINFORMATION|MB_ICONQUESTION|MB_ICONSTOP|MB_OK|MB_OKCANCEL|MB_RETRYCANCEL|",
                            r"MB_RIGHT|MB_RTLREADING|MB_SETFOREGROUND|MB_TOPMOST|MB_USERICON|MB_YESNO|MB_YESNOCANCEL|NORMAL|",
                            r"OFFLINE|READONLY|SHCTX|SHELL_CONTEXT|SW_HIDE|SW_SHOW|SW_SHOWDEFAULT|SW_SHOWMAXIMIZED|",
                            r"SW_SHOWMINIMIZED|SW_SHOWNORMAL|SYSTEM|TEMPORARY)\b",
                        ),
                        "atom",
                    )
                },
                SimpleRule {
                    ignore_case: true,
                    ..styled(
                        concat!(
                            r"\b(?:admin|all|amd64-unicode|auto|both|bottom|bzip2|components|current|custom|default|directory|",
                            r"false|force|hide|highest|ifdiff|ifnewer|instfiles|lastused|leave|left|license|listonly|lzma|",
                            r"nevershow|none|normal|notset|off|on|right|show|silent|silentlog|textonly|top|true|try|",
                            r"un\.components|un\.custom|un\.directory|un\.instfiles|un\.license|uninstConfirm|user|Win10|Win7|",
                            r"Win8|Win8\.1|WinVista|x86-(ansi|unicode)|zlib)\b",
                        ),
                        "builtin",
                    )
                },
                // LogicLib.nsh
                SimpleRule {
                    ignore_case: true,
                    indent: true,
                    ..styled(
                        concat!(
                            r"\$\{(?:And(?:If(?:Not)?|Unless)|Break|Case(?:2|3|4|5|Else)?|Continue|Default|Do(?:Until|While)?|",
                            r"Else(?:If(?:Not)?|Unless)?|End(?:If|Select|Switch)|Exit(?:Do|For|While)|For(?:Each)?|If(?:Cmd|",
                            r"Not(?:Then)?|Then)?|Loop(?:Until|While)?|Or(?:If(?:Not)?|Unless)|Select|Switch|Unless|While)\}",
                        ),
                        "variable-2",
                    )
                },
                // FileFunc.nsh
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(
                        concat!(
                            r"\$\{(?:BannerTrimPath|DirState|DriveSpace|Get(BaseName|Drives|ExeName|ExePath|FileAttributes|",
                            r"FileExt|FileName|FileVersion|Options|OptionsS|Parameters|Parent|Root|Size|Time)|Locate|",
                            r"RefreshShellIcons)\}",
                        ),
                        "variable-2",
                    )
                },
                // Memento.nsh
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(
                        r"\$\{(?:Memento(?:Section(?:Done|End|Restore|Save)?|UnselectedSection))\}",
                        "variable-2",
                    )
                },
                // TextFunc.nsh
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(
                        r"\$\{(?:Config(?:Read|ReadS|Write|WriteS)|File(?:Join|ReadFromEnd|Recode)|Line(?:Find|Read|Sum)|Text(?:Compare|CompareS)|TrimNewLines)\}",
                        "variable-2",
                    )
                },
                // WinVer.nsh
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(
                        r"\$\{(?:(?:At(?:Least|Most)|Is)(?:ServicePack|Win(?:7|8|8\.1|10|11|95|98|200(?:0|3|8(?:R2)?)|ME|NT4|Vista|XP))|Is(?:NT|Server))\}",
                        "variable",
                    )
                },
                // WordFunc.nsh
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(
                        r"\$\{(?:StrFilterS?|Version(?:Compare|Convert)|Word(?:AddS?|Find(?:(?:2|3)X)?S?|InsertS?|ReplaceS?))\}",
                        "keyword",
                    )
                },
                // x64.nsh
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(r"\$\{(?:RunningX64)\}", "variable")
                },
                SimpleRule {
                    ignore_case: true,
                    dedent: true,
                    ..styled(r"\$\{(?:Disable|Enable)X64FSRedirection\}", "keyword")
                },
                // Line Comment
                styled(r"(#|;).*", "comment"),
                // Block Comment
                next(styled(r"/\*", "comment"), "comment"),
                // Operator
                styled(r"[-+/*=<>!]+", "operator"),
                // Variable
                styled(r"\$\w[\w\.]*", "variable"),
                // Constant
                styled(r"\${[\!\w\.:-]+}", "variableName.constant"),
                // Language String
                styled(r"\$\([\!\w\.:-]+\)", "atom"),
            ],
        ),
        (
            "comment",
            vec![
                next(styled(r".*?\*/", "comment"), "start"),
                styled(r".*", "comment"),
            ],
        ),
    ]
}

pub(super) fn indent_on_input(line: &str) -> bool {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::with_flags(r"^\s*((Function|PageEx|Section|Section(Group)?)End|(\!(endif|macroend))|\$\{(End(If|Unless|While)|Loop(Until)|Next)\})$", "i").expect("valid NSIS indentation pattern")
    });
    PATTERN.find(line).is_some()
}
