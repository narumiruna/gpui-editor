// Word lists from the SRI-verified @codemirror/legacy-modes 6.5.5 mode/clike.js.
// See docs/licenses/codemirror-legacy-modes.txt.

use super::LegacyMode;

pub(super) struct Spec {
    pub name: &'static str,
    pub keywords: &'static str,
    pub types: &'static str,
    pub builtins: &'static str,
    pub atoms: &'static str,
    pub block_keywords: &'static str,
    pub def_keywords: &'static str,
}

macro_rules! c_keywords {
    () => {
        "auto if break case register continue return default do sizeof static else struct switch \
        extern typedef union for goto while enum const volatile inline restrict asm fortran"
    };
}

macro_rules! c_block_keywords {
    () => {
        "case do else for if switch while struct enum union"
    };
}

macro_rules! c_def_keywords {
    () => {
        "struct enum union"
    };
}

pub(super) fn spec(mode: LegacyMode) -> &'static Spec {
    match mode {
        LegacyMode::C => &Spec {
            name: "c",
            keywords: c_keywords!(),
            types: "",
            builtins: "",
            atoms: "NULL true false",
            block_keywords: c_block_keywords!(),
            def_keywords: c_def_keywords!(),
        },
        LegacyMode::Cpp => &Spec {
            name: "cpp",
            keywords: concat!(
                c_keywords!(),
                " ",
                "alignas alignof and and_eq audit axiom bitand bitor catch class compl concept \
            constexpr const_cast decltype delete dynamic_cast explicit export final friend import \
            module mutable namespace new noexcept not not_eq operator or or_eq override private \
            protected public reinterpret_cast requires static_assert static_cast template this \
            thread_local throw try typeid typename using virtual xor xor_eq"
            ),
            types: "",
            builtins: "",
            atoms: "true false NULL nullptr",
            block_keywords: concat!(c_block_keywords!(), " ", "class try catch"),
            def_keywords: concat!(c_def_keywords!(), " ", "class namespace"),
        },
        LegacyMode::Java => &Spec {
            name: "java",
            keywords: "abstract assert break case catch class const continue default do else enum extends \
            final finally for goto if implements import instanceof interface native new package \
            private protected public return static strictfp super switch synchronized this throw \
            throws transient try volatile while @interface",
            types: "var byte short int long float double boolean char void Boolean Byte Character Double \
            Float Integer Long Number Object Short String StringBuffer StringBuilder Void",
            builtins: "",
            atoms: "true false null",
            block_keywords: "catch class do else finally for if switch try while",
            def_keywords: "class interface enum @interface",
        },
        LegacyMode::CSharp => &Spec {
            name: "csharp",
            keywords: "abstract as async await base break case catch checked class const continue default \
            delegate do else enum event explicit extern finally fixed for foreach goto if \
            implicit in init interface internal is lock namespace new operator out override \
            params private protected public readonly record ref required return sealed sizeof \
            stackalloc static struct switch this throw try typeof unchecked unsafe using virtual \
            void volatile while add alias ascending descending dynamic from get global group into \
            join let orderby partial remove select set value var yield",
            types: "Action Boolean Byte Char DateTime DateTimeOffset Decimal Double Func Guid Int16 Int32 \
            Int64 Object SByte Single String Task TimeSpan UInt16 UInt32 UInt64 bool byte char \
            decimal double short int long object sbyte float string ushort uint ulong",
            builtins: "",
            atoms: "true false null",
            block_keywords: "catch class do else finally for foreach if struct switch try while",
            def_keywords: "class interface namespace record struct var",
        },
        LegacyMode::Scala => &Spec {
            name: "scala",
            keywords: "abstract case catch class def do else extends final finally for forSome if implicit \
            import lazy match new null object override package private protected return sealed \
            super this throw trait try type val var while with yield _ assert assume require \
            print println printf readLine readBoolean readByte readShort readChar readInt \
            readLong readFloat readDouble",
            types: "AnyVal App Application Array BufferedIterator BigDecimal BigInt Char Console Either \
            Enumeration Equiv Error Exception Fractional Function IndexedSeq Int Integral \
            Iterable Iterator List Map Numeric Nil NotNull Option Ordered Ordering \
            PartialFunction PartialOrdering Product Proxy Range Responder Seq Serializable Set \
            Specializable Stream StringBuilder StringContext Symbol Throwable Traversable \
            TraversableOnce Tuple Unit Vector Boolean Byte Character CharSequence Class \
            ClassLoader Cloneable Comparable Compiler Double Float Integer Long Math Number \
            Object Package Pair Process Runtime Runnable SecurityManager Short StackTraceElement \
            StrictMath String StringBuffer System Thread ThreadGroup ThreadLocal Triple Void",
            builtins: "",
            atoms: "true false null",
            block_keywords: "catch class enum do else finally for forSome if match switch try while",
            def_keywords: "class enum def object package trait type val var",
        },
        LegacyMode::Kotlin => &Spec {
            name: "kotlin",
            keywords: "package as typealias class interface this super val operator var fun for is in This \
            throw return annotation break continue object if else while do try when !in !is as? \
            file import where by get set abstract enum open inner override private public \
            internal protected catch finally out final vararg reified dynamic companion \
            constructor init sealed field property receiver param sparam lateinit data inline \
            noinline tailrec external crossinline const infix suspend actual expect setparam",
            types: "Boolean Byte Character CharSequence Class ClassLoader Cloneable Comparable Compiler \
            Double Exception Float Integer Long Math Number Object Package Pair Process Runtime \
            Runnable SecurityManager Short StackTraceElement StrictMath String StringBuffer \
            System Thread ThreadGroup ThreadLocal Throwable Triple Void Annotation Any \
            BooleanArray ByteArray Char CharArray DeprecationLevel DoubleArray Enum FloatArray \
            Function Int IntArray Lazy LazyThreadSafetyMode LongArray Nothing ShortArray Unit",
            builtins: "",
            atoms: "true false null this",
            block_keywords: "catch class do else finally for if where try while enum",
            def_keywords: "class val var object interface fun",
        },
        LegacyMode::Shader => &Spec {
            name: "shader",
            keywords: "sampler1D sampler2D sampler3D samplerCube sampler1DShadow sampler2DShadow const \
            attribute uniform varying break continue discard return for while do if else struct \
            in out inout",
            types: "float int bool void vec2 vec3 vec4 ivec2 ivec3 ivec4 bvec2 bvec3 bvec4 mat2 mat3 mat4",
            builtins: "radians degrees sin cos tan asin acos atan pow exp log exp2 sqrt inversesqrt abs sign \
            floor ceil fract mod min max clamp mix step smoothstep length distance dot cross \
            normalize ftransform faceforward reflect refract matrixCompMult lessThan \
            lessThanEqual greaterThan greaterThanEqual equal notEqual any all not texture1D \
            texture1DProj texture1DLod texture1DProjLod texture2D texture2DProj texture2DLod \
            texture2DProjLod texture3D texture3DProj texture3DLod texture3DProjLod textureCube \
            textureCubeLod shadow1D shadow2D shadow1DProj shadow2DProj shadow1DLod shadow2DLod \
            shadow1DProjLod shadow2DProjLod dFdx dFdy fwidth noise1 noise2 noise3 noise4",
            atoms: "true false gl_FragColor gl_SecondaryColor gl_Normal gl_Vertex gl_MultiTexCoord0 \
            gl_MultiTexCoord1 gl_MultiTexCoord2 gl_MultiTexCoord3 gl_MultiTexCoord4 \
            gl_MultiTexCoord5 gl_MultiTexCoord6 gl_MultiTexCoord7 gl_FogCoord gl_PointCoord \
            gl_Position gl_PointSize gl_ClipVertex gl_FrontColor gl_BackColor \
            gl_FrontSecondaryColor gl_BackSecondaryColor gl_TexCoord gl_FogFragCoord gl_FragCoord \
            gl_FrontFacing gl_FragData gl_FragDepth gl_ModelViewMatrix gl_ProjectionMatrix \
            gl_ModelViewProjectionMatrix gl_TextureMatrix gl_NormalMatrix \
            gl_ModelViewMatrixInverse gl_ProjectionMatrixInverse \
            gl_ModelViewProjectionMatrixInverse gl_TextureMatrixTranspose \
            gl_ModelViewMatrixInverseTranspose gl_ProjectionMatrixInverseTranspose \
            gl_ModelViewProjectionMatrixInverseTranspose gl_TextureMatrixInverseTranspose \
            gl_NormalScale gl_DepthRange gl_ClipPlane gl_Point gl_FrontMaterial gl_BackMaterial \
            gl_LightSource gl_LightModel gl_FrontLightModelProduct gl_BackLightModelProduct \
            gl_TextureColor gl_EyePlaneS gl_EyePlaneT gl_EyePlaneR gl_EyePlaneQ gl_FogParameters \
            gl_MaxLights gl_MaxClipPlanes gl_MaxTextureUnits gl_MaxTextureCoords \
            gl_MaxVertexAttribs gl_MaxVertexUniformComponents gl_MaxVaryingFloats \
            gl_MaxVertexTextureImageUnits gl_MaxTextureImageUnits gl_MaxFragmentUniformComponents \
            gl_MaxCombineTextureImageUnits gl_MaxDrawBuffers",
            block_keywords: "for while do if else struct",
            def_keywords: "",
        },
        LegacyMode::NesC => &Spec {
            name: "nesc",
            keywords: concat!(
                c_keywords!(),
                " ",
                "as atomic async call command component components configuration event generic \
            implementation includes interface module new norace nx_struct nx_union post provides \
            signal task uses abstract extends"
            ),
            types: "",
            builtins: "",
            atoms: "null true false",
            block_keywords: c_block_keywords!(),
            def_keywords: "",
        },
        LegacyMode::ObjectiveC => &Spec {
            name: "objectivec",
            keywords: concat!(
                c_keywords!(),
                " ",
                "bycopy byref in inout oneway out self super atomic nonatomic retain copy readwrite \
            readonly strong weak assign typeof nullable nonnull null_resettable _cmd @interface \
            @implementation @end @protocol @encode @property @synthesize @dynamic @class @public \
            @package @private @protected @required @optional @try @catch @finally @import \
            @selector @defs @synchronized @autoreleasepool @compatibility_alias @available"
            ),
            types: "",
            builtins: "FOUNDATION_EXPORT FOUNDATION_EXTERN NS_INLINE NS_FORMAT_FUNCTION  \
            NS_RETURNS_RETAINEDNS_ERROR_ENUM NS_RETURNS_NOT_RETAINED NS_RETURNS_INNER_POINTER \
            NS_DESIGNATED_INITIALIZER NS_ENUM NS_OPTIONS NS_REQUIRES_NIL_TERMINATION \
            NS_ASSUME_NONNULL_BEGIN NS_ASSUME_NONNULL_END NS_SWIFT_NAME NS_REFINED_FOR_SWIFT",
            atoms: "YES NO NULL Nil nil true false nullptr",
            block_keywords: concat!(
                c_block_keywords!(),
                " ",
                "@synthesize @try @catch @finally @autoreleasepool @synchronized"
            ),
            def_keywords: concat!(
                c_def_keywords!(),
                " ",
                "@interface @implementation @protocol @class"
            ),
        },
        LegacyMode::ObjectiveCpp => &Spec {
            name: "objectivecpp",
            keywords: concat!(
                c_keywords!(),
                " ",
                "bycopy byref in inout oneway out self super atomic nonatomic retain copy readwrite \
            readonly strong weak assign typeof nullable nonnull null_resettable _cmd @interface \
            @implementation @end @protocol @encode @property @synthesize @dynamic @class @public \
            @package @private @protected @required @optional @try @catch @finally @import \
            @selector @defs @synchronized @autoreleasepool @compatibility_alias @available \
            alignas alignof and and_eq audit axiom bitand bitor catch class compl concept \
            constexpr const_cast decltype delete dynamic_cast explicit export final friend import \
            module mutable namespace new noexcept not not_eq operator or or_eq override private \
            protected public reinterpret_cast requires static_assert static_cast template this \
            thread_local throw try typeid typename using virtual xor xor_eq"
            ),
            types: "",
            builtins: "FOUNDATION_EXPORT FOUNDATION_EXTERN NS_INLINE NS_FORMAT_FUNCTION  \
            NS_RETURNS_RETAINEDNS_ERROR_ENUM NS_RETURNS_NOT_RETAINED NS_RETURNS_INNER_POINTER \
            NS_DESIGNATED_INITIALIZER NS_ENUM NS_OPTIONS NS_REQUIRES_NIL_TERMINATION \
            NS_ASSUME_NONNULL_BEGIN NS_ASSUME_NONNULL_END NS_SWIFT_NAME NS_REFINED_FOR_SWIFT",
            atoms: "YES NO NULL Nil nil true false nullptr",
            block_keywords: concat!(
                c_block_keywords!(),
                " ",
                "@synthesize @try @catch @finally @autoreleasepool @synchronized class try catch"
            ),
            def_keywords: concat!(
                c_def_keywords!(),
                " ",
                "@interface @implementation @protocol @class class namespace"
            ),
        },
        LegacyMode::Squirrel => &Spec {
            name: "squirrel",
            keywords: "base break clone continue const default delete enum extends function in class foreach \
            local resume return this throw typeof yield constructor instanceof static",
            types: "",
            builtins: "",
            atoms: "true false null",
            block_keywords: "case catch class else for foreach if switch try while",
            def_keywords: "function local class",
        },
        LegacyMode::Ceylon => &Spec {
            name: "ceylon",
            keywords: "abstracts alias assembly assert assign break case catch class continue dynamic else \
            exists extends finally for function given if import in interface is let module new \
            nonempty object of out outer package return satisfies super switch then this throw \
            try value void while",
            types: "",
            builtins: "abstract actual aliased annotation by default deprecated doc final formal late \
            license native optional sealed see serializable shared suppressWarnings tagged throws \
            variable",
            atoms: "true false null larger smaller equal empty finished",
            block_keywords: "case catch class dynamic else finally for function if interface module new object \
            switch try while",
            def_keywords: "class dynamic function interface module object package value",
        },
        LegacyMode::Dart => &Spec {
            name: "dart",
            keywords: "this super static final const abstract class extends external factory implements \
            mixin get native set typedef with enum throw rethrow assert break case continue \
            default in return new deferred async await covariant try catch finally do else for if \
            switch while import library export part of show hide is as extension on yield late \
            required sealed base interface when inline",
            types: "",
            builtins: "void bool num int double dynamic var String Null Never",
            atoms: "true false null",
            block_keywords: "try catch finally do else for if switch while",
            def_keywords: "",
        },
        _ => unreachable!("not a C-like mode"),
    }
}
