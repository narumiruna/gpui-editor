// Word lists from the verified TTCN exports. Completion preserves upstream order.

pub(super) const KEYWORDS: &str = "activate address alive all alt altstep and and4b any break case component const continue \
    control deactivate display do else encode enumerated except exception execute extends \
    extension external for from function goto group if import in infinity inout interleave \
    label language length log match message mixed mod modifies module modulepar mtc noblock \
    not not4b nowait of on optional or or4b out override param pattern port procedure record \
    recursive rem repeat return runs select self sender set signature system template testcase \
    to type union value valueof var variant while with xor xor4b";
pub(super) const BUILTIN: &str = "bit2hex bit2int bit2oct bit2str char2int char2oct encvalue decomp decvalue float2int \
    float2str hex2bit hex2int hex2oct hex2str int2bit int2char int2float int2hex int2oct \
    int2str int2unichar isbound ischosen ispresent isvalue lengthof log2str oct2bit oct2char \
    oct2hex oct2int oct2str regexp replace rnd sizeof str2bit str2float str2hex str2int \
    str2oct substr unichar2int unichar2char enum2int";
pub(super) const TIMER_OPS: &str = "read running start stop timeout";
pub(super) const PORT_OPS: &str =
    "call catch check clear getcall getreply halt raise receive reply send trigger";

pub(super) const TTCN: &[(&str, &str)] = &[
    (KEYWORDS, "keyword"),
    (BUILTIN, "builtin"),
    (TIMER_OPS, "def"),
    (PORT_OPS, "def"),
    (
        "create connect disconnect done kill killed map unmap getverdict setverdict action \
        apply derefers refers",
        "def",
    ),
    (
        "error fail inconc none pass true false null NULL omit",
        "string",
    ),
    (
        "anytype bitstring boolean char charstring default float hexstring integer objid \
        octetstring universal verdicttype timer",
        "typeName.standard",
    ),
    ("private public friend", "modifier"),
    ("complement ifpresent subset superset permutation", "atom"),
];

pub(super) const CONFIG: &[(&str, &str)] = &[
    (
        "Yes No LogFile FileMask ConsoleMask AppendFile TimeStampFormat LogEventTypes \
        SourceInfoFormat LogEntityName LogSourceInfo DiskFullAction LogFileNumber LogFileSize \
        MatchingHints Detailed Compact SubCategories Stack Single None Seconds DateTime Time \
        Stop Error Retry Delete TCPPort KillTimer NumHCs UnixSocketsEnabled LocalAddress",
        "keyword",
    ),
    (
        "TTCN_EXECUTOR TTCN_ERROR TTCN_WARNING TTCN_PORTEVENT TTCN_TIMEROP TTCN_VERDICTOP \
        TTCN_DEFAULTOP TTCN_TESTCASE TTCN_ACTION TTCN_USER TTCN_FUNCTION TTCN_STATISTICS \
        TTCN_PARALLEL TTCN_MATCHING TTCN_DEBUG EXECUTOR ERROR WARNING PORTEVENT TIMEROP \
        VERDICTOP DEFAULTOP TESTCASE ACTION USER FUNCTION STATISTICS PARALLEL MATCHING DEBUG \
        LOG_ALL LOG_NOTHING ACTION_UNQUALIFIED DEBUG_ENCDEC DEBUG_TESTPORT DEBUG_UNQUALIFIED \
        DEFAULTOP_ACTIVATE DEFAULTOP_DEACTIVATE DEFAULTOP_EXIT DEFAULTOP_UNQUALIFIED \
        ERROR_UNQUALIFIED EXECUTOR_COMPONENT EXECUTOR_CONFIGDATA EXECUTOR_EXTCOMMAND \
        EXECUTOR_LOGOPTIONS EXECUTOR_RUNTIME EXECUTOR_UNQUALIFIED FUNCTION_RND \
        FUNCTION_UNQUALIFIED MATCHING_DONE MATCHING_MCSUCCESS MATCHING_MCUNSUCC \
        MATCHING_MMSUCCESS MATCHING_MMUNSUCC MATCHING_PCSUCCESS MATCHING_PCUNSUCC \
        MATCHING_PMSUCCESS MATCHING_PMUNSUCC MATCHING_PROBLEM MATCHING_TIMEOUT \
        MATCHING_UNQUALIFIED PARALLEL_PORTCONN PARALLEL_PORTMAP PARALLEL_PTC \
        PARALLEL_UNQUALIFIED PORTEVENT_DUALRECV PORTEVENT_DUALSEND PORTEVENT_MCRECV \
        PORTEVENT_MCSEND PORTEVENT_MMRECV PORTEVENT_MMSEND PORTEVENT_MQUEUE PORTEVENT_PCIN \
        PORTEVENT_PCOUT PORTEVENT_PMIN PORTEVENT_PMOUT PORTEVENT_PQUEUE PORTEVENT_STATE \
        PORTEVENT_UNQUALIFIED STATISTICS_UNQUALIFIED STATISTICS_VERDICT TESTCASE_FINISH \
        TESTCASE_START TESTCASE_UNQUALIFIED TIMEROP_GUARD TIMEROP_READ TIMEROP_START \
        TIMEROP_STOP TIMEROP_TIMEOUT TIMEROP_UNQUALIFIED USER_UNQUALIFIED VERDICTOP_FINAL \
        VERDICTOP_GETVERDICT VERDICTOP_SETVERDICT VERDICTOP_UNQUALIFIED WARNING_UNQUALIFIED",
        "atom",
    ),
    (
        "BeginControlPart EndControlPart BeginTestCase EndTestCase",
        "deleted",
    ),
];
