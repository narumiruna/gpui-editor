use std::{collections::HashSet, sync::LazyLock};

use super::{
    FUNCTIONS, Params, ends_comment, ends_region, number_tail, operator, starts_region, word,
};
use crate::{
    legacy_modes::{js_line_terminator, js_space},
    stream_parser::StringStream,
};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    phase: Phase,
    params: Params,
}

#[derive(Clone, Copy, Debug, Default)]
enum Phase {
    #[default]
    Base,
    Comment,
    Region,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        self.scan(stream)
    }

    fn scan(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.phase {
            Phase::Base => self.base(stream),
            Phase::Comment => {
                if ends_comment(stream, '/') {
                    self.phase = Phase::Base;
                }
                Some("comment")
            }
            Phase::Region => {
                if ends_region(stream, ')', ';') {
                    self.phase = Phase::Base;
                }
                Some("meta")
            }
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        static COMMANDS: LazyLock<HashSet<&str>> =
            LazyLock::new(|| KEYWORDS.split_ascii_whitespace().collect());
        static IDENTIFIERS: LazyLock<HashSet<&str>> =
            LazyLock::new(|| SPECIALS.split_ascii_whitespace().collect());

        let before = std::mem::take(&mut self.params.before);
        let ch = stream.next()?;
        match ch {
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | '.' => {
                self.params.punctuation(ch, before);
                None
            }
            '0'..='9' => {
                stream.eat_while(number_tail);
                Some("number")
            }
            '\\' => {
                stream.eat('\\');
                stream.eat_if(|ch| !js_line_terminator(ch));
                Some("number")
            }
            '/' if stream.eat('*').is_some() => {
                self.phase = Phase::Comment;
                self.scan(stream)
            }
            ';' if starts_region(stream, '(') => {
                self.phase = Phase::Region;
                self.scan(stream)
            }
            ';' if !self.params.inside => {
                stream.skip_to_end();
                Some("comment")
            }
            '"' => {
                stream.eat('"');
                Some("keyword")
            }
            '$' => {
                stream.eat_while(|ch| word(ch) || "$.:".contains(ch));
                if IDENTIFIERS.contains(stream.current().to_ascii_lowercase().as_str()) {
                    Some("keyword")
                } else {
                    self.params.before = true;
                    Some("builtin")
                }
            }
            '%' => {
                stream.eat_while(|ch| !js_space(ch) && !",()".contains(ch));
                self.params.before = true;
                Some("string")
            }
            ch if operator(ch) => {
                stream.eat_while(operator);
                Some("operator")
            }
            _ => {
                stream.eat_while(|ch| word(ch) || "${}".contains(ch));
                let command = stream.current().to_lowercase();
                if COMMANDS.contains(command.as_str()) {
                    Some("keyword")
                } else if FUNCTIONS.contains(&command.as_str()) {
                    self.params.before = true;
                    Some("keyword")
                } else {
                    None
                }
            }
        }
    }
}

const SPECIALS: &str = "\
    $! $$ $& $? $+ $abook $abs $active $activecid \
    $activewid $address $addtok $agent $agentname $agentstat $agentver \
    $alias $and $anick $ansi2mirc $aop $appactive $appstate $asc $asctime \
    $asin $atan $avoice $away $awaymsg $awaytime $banmask $base $bfind \
    $binoff $biton $bnick $bvar $bytes $calc $cb $cd $ceil $chan $chanmodes \
    $chantypes $chat $chr $cid $clevel $click $cmdbox $cmdline $cnick $color \
    $com $comcall $comchan $comerr $compact $compress $comval $cos $count \
    $cr $crc $creq $crlf $ctime $ctimer $ctrlenter $date $day $daylight \
    $dbuh $dbuw $dccignore $dccport $dde $ddename $debug $decode $decompress \
    $deltok $devent $dialog $did $didreg $didtok $didwm $disk $dlevel $dll \
    $dllcall $dname $dns $duration $ebeeps $editbox $emailaddr $encode $error \
    $eval $event $exist $feof $ferr $fgetc $file $filename $filtered $finddir \
    $finddirn $findfile $findfilen $findtok $fline $floor $fopen $fread $fserve \
    $fulladdress $fulldate $fullname $fullscreen $get $getdir $getdot $gettok $gmt \
    $group $halted $hash $height $hfind $hget $highlight $hnick $hotline \
    $hotlinepos $ial $ialchan $ibl $idle $iel $ifmatch $ignore $iif $iil \
    $inelipse $ini $inmidi $inpaste $inpoly $input $inrect $inroundrect \
    $insong $instok $int $inwave $ip $isalias $isbit $isdde $isdir $isfile \
    $isid $islower $istok $isupper $keychar $keyrpt $keyval $knick $lactive \
    $lactivecid $lactivewid $left $len $level $lf $line $lines $link $lock \
    $lock $locked $log $logstamp $logstampfmt $longfn $longip $lower $ltimer \
    $maddress $mask $matchkey $matchtok $md5 $me $menu $menubar $menucontext \
    $menutype $mid $middir $mircdir $mircexe $mircini $mklogfn $mnick $mode \
    $modefirst $modelast $modespl $mouse $msfile $network $newnick $nick $nofile \
    $nopath $noqt $not $notags $notify $null $numeric $numok $oline $onpoly \
    $opnick $or $ord $os $passivedcc $pic $play $pnick $port $portable $portfree \
    $pos $prefix $prop $protect $puttok $qt $query $rand $r $rawmsg $read $readomo \
    $readn $regex $regml $regsub $regsubex $remove $remtok $replace $replacex \
    $reptok $result $rgb $right $round $scid $scon $script $scriptdir $scriptline \
    $sdir $send $server $serverip $sfile $sha1 $shortfn $show $signal $sin \
    $site $sline $snick $snicks $snotify $sock $sockbr $sockerr $sockname \
    $sorttok $sound $sqrt $ssl $sreq $sslready $status $strip $str $stripped \
    $syle $submenu $switchbar $tan $target $ticks $time $timer $timestamp \
    $timestampfmt $timezone $tip $titlebar $toolbar $treebar $trust $ulevel \
    $ulist $upper $uptime $url $usermode $v1 $v2 $var $vcmd $vcmdstat $vcmdver \
    $version $vnick $vol $wid $width $wildsite $wildtok $window $wrap $xor";

const KEYWORDS: &str = "\
    abook ajinvite alias aline ame amsg anick aop auser autojoin avoice \
    away background ban bcopy beep bread break breplace bset btrunc bunset bwrite \
    channel clear clearall cline clipboard close cnick color comclose comopen \
    comreg continue copy creq ctcpreply ctcps dcc dccserver dde ddeserver \
    debug dec describe dialog did didtok disable disconnect dlevel dline dll \
    dns dqwindow drawcopy drawdot drawfill drawline drawpic drawrect drawreplace \
    drawrot drawsave drawscroll drawtext ebeeps echo editbox emailaddr enable \
    events exit fclose filter findtext finger firewall flash flist flood flush \
    flushini font fopen fseek fsend fserve fullname fwrite ghide gload gmove \
    gopts goto gplay gpoint gqreq groups gshow gsize gstop gtalk gunload hadd \
    halt haltdef hdec hdel help hfree hinc hload hmake hop hsave ial ialclear \
    ialmark identd if ignore iline inc invite iuser join kick linesep links list \
    load loadbuf localinfo log mdi me menubar mkdir mnick mode msg nick noop notice \
    notify omsg onotice part partall pdcc perform play playctrl pop protect pvoice \
    qme qmsg query queryn quit raw reload remini remote remove rename renwin \
    reseterror resetidle return rlevel rline rmdir run ruser save savebuf saveini \
    say scid scon server set showmirc signam sline sockaccept sockclose socklist \
    socklisten sockmark sockopen sockpause sockread sockrename sockudp sockwrite \
    sound speak splay sreq strip switchbar timer timestamp titlebar tnick tokenize \
    toolbar topic tray treebar ulist unload unset unsetall updatenl url uwho \
    var vcadd vcmd vcrem vol while whois window winhelp write writeint if isalnum \
    isalpha isaop isavoice isban ischan ishop isignore isin isincs isletter islower \
    isnotify isnum ison isop isprotect isreg isupper isvoice iswm iswmcs \
    elseif else goto menu nicklist status title icon size option text edit \
    button check radio box scroll list combo link tab item";
