#chg-compatible
#require chg linux

  $ configure modernclient
  $ newclientrepo
  $ drawdag <<'EOS'
  > B
  > |
  > A
  > EOS
  $ hg go -q $B
  $ CHGDEBUG=1 hg files 2>&1 | grep 'request runcommand'
  chg: debug: *request runcommand* (glob)
