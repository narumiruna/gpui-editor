// Word lists and dialect options from @codemirror/lang-sql 6.10.0.
// See docs/licenses/codemirror-lang-sql.txt.

use super::SqlDialectSpec;

macro_rules! sql_keywords {
    ($($extra:literal),*) => {
        concat!("absolute action add after all allocate alter and any are as asc assertion at authorization \
            before begin between both breadth by call cascade cascaded case cast catalog check close \
            collate collation column commit condition connect connection constraint constraints constructor \
            continue corresponding count create cross cube current current_date \
            current_default_transform_group current_transform_group_for_type current_path current_role \
            current_time current_timestamp current_user cursor cycle data day deallocate declare default \
            deferrable deferred delete depth deref desc describe descriptor deterministic diagnostics \
            disconnect distinct do domain drop dynamic each else elseif end end-exec equals escape except \
            exception exec execute exists exit external fetch first for foreign found from free full \
            function general get global go goto grant group grouping handle having hold hour identity if \
            immediate in indicator initially inner inout input insert intersect into is isolation join key \
            language last lateral leading leave left level like limit local localtime localtimestamp \
            locator loop map match method minute modifies module month names natural nesting new next no \
            none not of old on only open option or order ordinality out outer output overlaps pad parameter \
            partial path prepare preserve primary prior privileges procedure public read reads recursive \
            redo ref references referencing relative release repeat resignal restrict result return returns \
            revoke right role rollback rollup routine row rows savepoint schema scroll search second \
            section select session session_user set sets signal similar size some space specific \
            specifictype sql sqlexception sqlstate sqlwarning start state static system_user table \
            temporary then timezone_hour timezone_minute to trailing transaction translation treat trigger \
            under undo union unique unnest until update usage user using value values view when whenever \
            where while with without work write year zone ", $($extra),*)
    };
}

macro_rules! sql_types {
    ($($extra:literal),*) => {
        concat!("array binary bit boolean char character clob date decimal double float int integer interval \
            large national nchar nclob numeric object precision real smallint time timestamp varchar \
            varying ", $($extra),*)
    };
}

macro_rules! mysql_keywords {
    ($($extra:literal),*) => {
        concat!("accessible algorithm analyze asensitive authors auto_increment autocommit avg avg_row_length \
            binlog btree cache catalog_name chain change changed checkpoint checksum class_origin \
            client_statistics coalesce code collations columns comment committed completion concurrent \
            consistent contains contributors convert database databases day_hour day_microsecond day_minute \
            day_second delay_key_write delayed delimiter des_key_file dev_pop dev_samp deviance directory \
            disable discard distinctrow div dual dumpfile enable enclosed ends engine engines enum errors \
            escaped even event events every explain extended fast field fields flush force found_rows \
            fulltext grants handler hash high_priority hosts hour_microsecond hour_minute hour_second \
            ignore ignore_server_ids import index index_statistics infile innodb insensitive insert_method \
            install invoker iterate keys kill linear lines list load lock logs low_priority master \
            master_heartbeat_period master_ssl_verify_server_cert masters max max_rows maxvalue \
            message_text middleint migrate min min_rows minute_microsecond minute_second mod mode modify \
            mutex mysql_errno no_write_to_binlog offline offset one online optimize optionally outfile \
            pack_keys parser partition partitions password phase plugin plugins prev processlist profile \
            profiles purge query quick range read_write rebuild recover regexp relaylog remove rename \
            reorganize repair repeatable replace require resume rlike row_format rtree schedule schema_name \
            schemas second_microsecond security sensitive separator serializable server share show slave \
            slow snapshot soname spatial sql_big_result sql_buffer_result sql_cache sql_calc_found_rows \
            sql_no_cache sql_small_result ssl starting starts std stddev stddev_pop stddev_samp storage \
            straight_join subclass_origin sum suspend table_name table_statistics tables tablespace \
            terminated triggers truncate uncommitted uninstall unlock upgrade use use_frm user_resources \
            user_statistics utc_date utc_time utc_timestamp variables views warnings xa xor year_month \
            zerofill", $($extra),*)
    };
}

pub(super) const STANDARD_KEYWORDS: &str = sql_keywords!();
pub(super) const STANDARD_TYPES: &str = sql_types!();

pub(super) const POSTGRESQL: SqlDialectSpec<'static> = SqlDialectSpec {
    char_set_casts: true,
    double_dollar_quoted_strings: true,
    operator_chars: "+-*/<>=~!@#%^&|`?",
    special_var: "",
    keywords: sql_keywords!(
        "abort abs absent access according ada admin aggregate alias also always analyse analyze \
        array_agg array_max_cardinality asensitive assert assignment asymmetric atomic attach attribute \
        attributes avg backward base64 begin_frame begin_partition bernoulli bit_length blocked bom \
        cache called cardinality catalog_name ceil ceiling chain char_length character_length \
        character_set_catalog character_set_name character_set_schema characteristics characters \
        checkpoint class class_origin cluster coalesce cobol collation_catalog collation_name \
        collation_schema collect column_name columns command_function command_function_code comment \
        comments committed concurrently condition_number configuration conflict connection_name \
        constant constraint_catalog constraint_name constraint_schema contains content control \
        conversion convert copy corr cost covar_pop covar_samp csv cume_dist current_catalog \
        current_row current_schema cursor_name database datalink datatype datetime_interval_code \
        datetime_interval_precision db debug defaults defined definer degree delimiter delimiters \
        dense_rank depends derived detach detail dictionary disable discard dispatch dlnewcopy \
        dlpreviouscopy dlurlcomplete dlurlcompleteonly dlurlcompletewrite dlurlpath dlurlpathonly \
        dlurlpathwrite dlurlscheme dlurlserver dlvalue document dump dynamic_function \
        dynamic_function_code element elsif empty enable encoding encrypted end_frame end_partition \
        endexec enforced enum errcode error event every exclude excluding exclusive exp explain \
        expression extension extract family file filter final first_value flag floor following force \
        foreach fortran forward frame_row freeze fs functions fusion generated granted greatest groups \
        handler header hex hierarchy hint id ignore ilike immediately immutable implementation implicit \
        import include including increment indent index indexes info inherit inherits inline \
        insensitive instance instantiable instead integrity intersection invoker isnull key_member \
        key_type label lag last_value lead leakproof least length library like_regex link listen ln \
        load location lock locked log logged lower mapping matched materialized max max_cardinality \
        maxvalue member merge message message_length message_octet_length message_text min minvalue mod \
        mode more move multiset mumps name namespace nfc nfd nfkc nfkd nil normalize normalized nothing \
        notice notify notnull nowait nth_value ntile nullable nullif nulls number occurrences_regex \
        octet_length octets off offset oids operator options ordering others over overlay overriding \
        owned owner parallel parameter_mode parameter_name parameter_ordinal_position \
        parameter_specific_catalog parameter_specific_name parameter_specific_schema parser partition \
        pascal passing passthrough password percent percent_rank percentile_cont percentile_disc \
        perform period permission pg_context pg_datatype_name pg_exception_context pg_exception_detail \
        pg_exception_hint placing plans pli policy portion position position_regex power precedes \
        preceding prepared print_strict_params procedural procedures program publication query quote \
        raise range rank reassign recheck recovery refresh regr_avgx regr_avgy regr_count \
        regr_intercept regr_r2 regr_slope regr_sxx regr_sxy regr_syy reindex rename repeatable replace \
        replica requiring reset respect restart restore result_oid returned_cardinality returned_length \
        returned_octet_length returned_sqlstate returning reverse routine_catalog routine_name \
        routine_schema routines row_count row_number rowtype rule scale schema_name schemas scope \
        scope_catalog scope_name scope_schema security selective self sensitive sequence sequences \
        serializable server server_name setof share show simple skip slice snapshot source \
        specific_name sqlcode sqlerror sqrt stable stacked standalone statement statistics stddev_pop \
        stddev_samp stdin stdout storage strict strip structure style subclass_origin submultiset \
        subscription substring substring_regex succeeds sum symmetric sysid system system_time \
        table_name tables tablesample tablespace temp template ties token top_level_count \
        transaction_active transactions_committed transactions_rolled_back transform transforms \
        translate translate_regex trigger_catalog trigger_name trigger_schema trim trim_array truncate \
        trusted type types uescape unbounded uncommitted unencrypted unlink unlisten unlogged unnamed \
        untyped upper uri use_column use_variable user_defined_type_catalog user_defined_type_code \
        user_defined_type_name user_defined_type_schema vacuum valid validate validator value_of \
        var_pop var_samp varbinary variable_conflict variadic verbose version versioning views volatile \
        warning whitespace width_bucket window within wrapper xmlagg xmlattributes xmlbinary xmlcast \
        xmlcomment xmlconcat xmldeclaration xmldocument xmlelement xmlexists xmlforest xmliterate \
        xmlnamespaces xmlparse xmlpi xmlquery xmlroot xmlschema xmlserialize xmltable xmltext \
        xmlvalidate yes"
    ),
    types: sql_types!(
        "bigint int8 bigserial serial8 varbit bool box bytea cidr circle precision float8 inet int4 json \
        jsonb line lseg macaddr macaddr8 money numeric pg_lsn point polygon float4 int2 smallserial \
        serial2 serial serial4 text timetz timestamptz tsquery tsvector txid_snapshot uuid xml"
    ),
    ..SqlDialectSpec::DEFAULT
};

pub(super) const MYSQL: SqlDialectSpec<'static> = SqlDialectSpec {
    operator_chars: "*+-%<>!=&|^",
    char_set_casts: true,
    double_quoted_strings: true,
    unquoted_bit_literals: true,
    hash_comments: true,
    space_after_dashes: true,
    special_var: "@?",
    identifier_quotes: "`",
    keywords: concat!(sql_keywords!("group_concat "), mysql_keywords!()),
    types: sql_types!(
        "bool blob long longblob longtext medium mediumblob mediumint mediumtext tinyblob tinyint \
        tinytext text bigint int1 int2 int3 int4 int8 float4 float8 varbinary varcharacter precision \
        datetime unsigned signed"
    ),
    builtin: "charset clear edit ego help nopager notee nowarning pager print prompt quit rehash source \
        status system tee",
    ..SqlDialectSpec::DEFAULT
};

pub(super) const MARIASQL: SqlDialectSpec<'static> = SqlDialectSpec {
    operator_chars: "*+-%<>!=&|^",
    char_set_casts: true,
    double_quoted_strings: true,
    unquoted_bit_literals: true,
    hash_comments: true,
    space_after_dashes: true,
    special_var: "@?",
    identifier_quotes: "`",
    keywords: concat!(
        sql_keywords!("always generated groupby_concat hard persistent shutdown soft virtual "),
        mysql_keywords!()
    ),
    types: sql_types!(
        "bool blob long longblob longtext medium mediumblob mediumint mediumtext tinyblob tinyint \
        tinytext text bigint int1 int2 int3 int4 int8 float4 float8 varbinary varcharacter precision \
        datetime unsigned signed"
    ),
    builtin: "charset clear edit ego help nopager notee nowarning pager print prompt quit rehash source \
        status system tee",
    ..SqlDialectSpec::DEFAULT
};

pub(super) const MSSQL: SqlDialectSpec<'static> = SqlDialectSpec {
    keywords: sql_keywords!(
        "add external procedure all fetch public alter file raiserror and fillfactor read any for \
        readtext as foreign reconfigure asc freetext references authorization freetexttable replication \
        backup from restore begin full restrict between function return break goto revert browse grant \
        revoke bulk group right by having rollback cascade holdlock rowcount case identity rowguidcol \
        check identity_insert rule checkpoint identitycol save close if schema clustered in \
        securityaudit coalesce index select collate inner semantickeyphrasetable column insert \
        semanticsimilaritydetailstable commit intersect semanticsimilaritytable compute into \
        session_user constraint is set contains join setuser containstable key shutdown continue kill \
        some convert left statistics create like system_user cross lineno table current load \
        tablesample current_date merge textsize current_time national then current_timestamp nocheck to \
        current_user nonclustered top cursor not tran database null transaction dbcc nullif trigger \
        deallocate of truncate declare off try_convert default offsets tsequal delete on union deny \
        open unique desc opendatasource unpivot disk openquery update distinct openrowset updatetext \
        distributed openxml use double option user drop or values dump order varying else outer view \
        end over waitfor errlvl percent when escape pivot where except plan while exec precision with \
        execute primary within group exists print writetext exit proc noexpand index forceseek \
        forcescan holdlock nolock nowait paglock readcommitted readcommittedlock readpast \
        readuncommitted repeatableread rowlock serializable snapshot spatial_window_max_cells tablock \
        tablockx updlock xlock keepidentity keepdefaults ignore_constraints ignore_triggers"
    ),
    types: sql_types!(
        "smalldatetime datetimeoffset datetime2 datetime bigint smallint smallmoney tinyint money real \
        text nvarchar ntext varbinary image hierarchyid uniqueidentifier sql_variant xml"
    ),
    builtin: "approx_count_distinct approx_percentile_cont approx_percentile_disc avg checksum_agg count \
        count_big grouping grouping_id max min product stdev stdevp sum var varp ai_generate_embeddings \
        ai_generate_chunks cume_dist first_value lag last_value lead percentile_cont percentile_disc \
        percent_rank left_shift right_shift bit_count get_bit set_bit collationproperty \
        tertiary_weights @@datefirst @@dbts @@langid @@language @@lock_timeout @@max_connections \
        @@max_precision @@nestlevel @@options @@remserver @@servername @@servicename @@spid @@textsize \
        @@version cast convert parse try_cast try_convert try_parse asymkey_id asymkeyproperty \
        certproperty cert_id crypt_gen_random decryptbyasymkey decryptbycert decryptbykey \
        decryptbykeyautoasymkey decryptbykeyautocert decryptbypassphrase encryptbyasymkey encryptbycert \
        encryptbykey encryptbypassphrase hashbytes is_objectsigned key_guid key_id key_name \
        signbyasymkey signbycert symkeyproperty verifysignedbycert verifysignedbyasymkey @@cursor_rows \
        @@fetch_status cursor_status datalength ident_current ident_incr ident_seed identity \
        sql_variant_property @@datefirst current_timestamp current_timezone current_timezone_id \
        date_bucket dateadd datediff datediff_big datefromparts datename datepart datetime2fromparts \
        datetimefromparts datetimeoffsetfromparts datetrunc day eomonth getdate getutcdate isdate month \
        smalldatetimefromparts switchoffset sysdatetime sysdatetimeoffset sysutcdatetime timefromparts \
        todatetimeoffset year edit_distance edit_distance_similarity jaro_winkler_distance \
        jaro_winkler_similarity edge_id_from_parts graph_id_from_edge_id graph_id_from_node_id \
        node_id_from_parts object_id_from_edge_id object_id_from_node_id json isjson json_array \
        json_contains json_modify json_object json_path_exists json_query json_value regexp_like \
        regexp_replace regexp_substr regexp_instr regexp_count regexp_matches regexp_split_to_table abs \
        acos asin atan atn2 ceiling cos cot degrees exp floor log log10 pi power radians rand round \
        sign sin sqrt square tan choose greatest iif least @@procid app_name applock_mode applock_test \
        assemblyproperty col_length col_name columnproperty databasepropertyex db_id db_name file_id \
        file_idex file_name filegroup_id filegroup_name filegroupproperty fileproperty filepropertyex \
        fulltextcatalogproperty fulltextserviceproperty index_col indexkey_property indexproperty next \
        value for object_definition object_id object_name object_schema_name objectproperty \
        objectpropertyex original_db_name parsename schema_id schema_name scope_identity serverproperty \
        stats_date type_id type_name typeproperty dense_rank ntile rank row_number publishingservername \
        certenclosed certprivatekey current_user database_principal_id has_dbaccess has_perms_by_name \
        is_member is_rolemember is_srvrolemember loginproperty original_login permissions pwdencrypt \
        pwdcompare session_user sessionproperty suser_id suser_name suser_sid suser_sname system_user \
        user user_id user_name ascii char charindex concat concat_ws difference format left len lower \
        ltrim nchar patindex quotename replace replicate reverse right rtrim soundex space str \
        string_agg string_escape stuff substring translate trim unicode upper $partition @@error \
        @@identity @@pack_received @@rowcount @@trancount binary_checksum checksum compress \
        connectionproperty context_info current_request_id current_transaction_id decompress error_line \
        error_message error_number error_procedure error_severity error_state formatmessage \
        get_filestream_transaction_context getansinull host_id host_name isnull isnumeric \
        min_active_rowversion newid newsequentialid rowcount_big session_context xact_state \
        @@connections @@cpu_busy @@idle @@io_busy @@pack_sent @@packet_errors @@timeticks \
        @@total_errors @@total_read @@total_write textptr textvalid columns_updated eventdata \
        trigger_nestlevel vector_distance vectorproperty vector_search generate_series opendatasource \
        openjson openquery openrowset openxml predict string_split coalesce nullif apply catch filter \
        force include keep keepfixed modify optimize parameterization parameters partition recompile \
        sequence set",
    operator_chars: "*+-%<>!=^&|/",
    special_var: "@",
    identifier_quotes: "\"[",
    ..SqlDialectSpec::DEFAULT
};

pub(super) const SQLITE: SqlDialectSpec<'static> = SqlDialectSpec {
    keywords: sql_keywords!(
        "abort analyze attach autoincrement conflict database detach exclusive fail glob ignore index \
        indexed instead isnull notnull offset plan pragma query raise regexp reindex rename replace \
        temp vacuum virtual"
    ),
    types: sql_types!(
        "bool blob long longblob longtext medium mediumblob mediumint mediumtext tinyblob tinyint \
        tinytext text bigint int2 int8 unsigned signed real"
    ),
    builtin: "auth backup bail changes clone databases dbinfo dump echo eqp explain fullschema headers help \
        import imposter indexes iotrace lint load log mode nullvalue once print prompt quit restore \
        save scanstats separator shell show stats system tables testcase timeout timer trace vfsinfo \
        vfslist vfsname width",
    operator_chars: "*+-%<>!=&|/~",
    identifier_quotes: "`\"",
    special_var: "@:?$",
    ..SqlDialectSpec::DEFAULT
};

pub(super) const CASSANDRA: SqlDialectSpec<'static> = SqlDialectSpec {
    keywords: "add all allow alter and any apply as asc authorize batch begin by clustering columnfamily \
        compact consistency count create custom delete desc distinct drop each_quorum exists filtering \
        from grant if in index insert into key keyspace keyspaces level limit local_one local_quorum \
        modify nan norecursive nosuperuser not of on one order password permission permissions primary \
        quorum rename revoke schema select set storage superuser table three to token truncate ttl two \
        type unlogged update use user users using values where with writetime infinity NaN",
    types: sql_types!(
        "ascii bigint blob counter frozen inet list map static text timeuuid tuple uuid varint"
    ),
    slash_comments: true,
    ..SqlDialectSpec::DEFAULT
};

pub(super) const PLSQL: SqlDialectSpec<'static> = SqlDialectSpec {
    keywords: sql_keywords!(
        "abort accept access add all alter and any arraylen as asc assert assign at attributes audit \
        authorization avg base_table begin between binary_integer body by case cast char_base check \
        close cluster clusters colauth column comment commit compress connected constant constraint \
        crash create current currval cursor data_base database dba deallocate debugoff debugon declare \
        default definition delay delete desc digits dispose distinct do drop else elseif elsif enable \
        end entry exception exception_init exchange exclusive exists external fast fetch file for force \
        form from function generic goto grant group having identified if immediate in increment index \
        indexes indicator initial initrans insert interface intersect into is key level library like \
        limited local lock log logging loop master maxextents maxtrans member minextents minus mislabel \
        mode modify multiset new next no noaudit nocompress nologging noparallel not nowait number_base \
        of off offline on online only option or order out package parallel partition pctfree \
        pctincrease pctused pls_integer positive positiven pragma primary prior private privileges \
        procedure public raise range raw rebuild record ref references refresh rename replace resource \
        restrict return returning returns reverse revoke rollback row rowid rowlabel rownum rows run \
        savepoint schema segment select separate set share snapshot some space split sql start \
        statement storage subtype successful synonym tabauth table tables tablespace task terminate \
        then to trigger truncate type union unique unlimited unrecoverable unusable update use using \
        validate value values variable view views when whenever where while with work"
    ),
    builtin: "appinfo arraysize autocommit autoprint autorecovery autotrace blockterminator break btitle \
        cmdsep colsep compatibility compute concat copycommit copytypecheck define echo editfile \
        embedded feedback flagger flush heading headsep instance linesize lno loboffset logsource \
        longchunksize markup native newpage numformat numwidth pagesize pause pno recsep recsepchar \
        repfooter repheader serveroutput shiftinout show showmode spool sqlblanklines sqlcase sqlcode \
        sqlcontinue sqlnumber sqlpluscompatibility sqlprefix sqlprompt sqlterminator suffix tab term \
        termout timing trimout trimspool ttitle underline verify version wrap",
    types: sql_types!(
        "ascii bfile bfilename bigserial bit blob dec long number nvarchar nvarchar2 serial smallint \
        string text uid varchar2 xml"
    ),
    operator_chars: "*/+-%<>!=~",
    double_quoted_strings: true,
    char_set_casts: true,
    plsql_quoting_mechanism: true,
    ..SqlDialectSpec::DEFAULT
};
