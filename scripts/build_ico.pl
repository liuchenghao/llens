use strict; use warnings;
my $out = shift;
my @sz  = (16, 32, 48, 64, 256);
my @src = @ARGV;  # 对应顺序的 5 个 png 文件
# 读各 png
my @png;
for my $f (@src) {
  open(my $fh, '<:raw', $f) or die "open $f: $!";
  local $/; my $d = <$fh>; close $fh;
  $d = '' unless defined $d;
  push @png, $d;
}
my $n = scalar @sz;
my $header = pack("vvv", 0, 1, $n);  # reserved,type,count
my $entries = '';
my $body = '';
my $offset = 6 + $n*16;
for my $i (0..$n-1) {
  my $w = $sz[$i]; $w = 0 if $w >= 256;
  # ICONDIRENTRY: w(1) h(1) colors(1)=0 reserved(1)=0 planes(2)=1 bitcount(2)=32 size(4) offset(4)
  $entries .= pack("CCCCvvII", $w, $w, 0, 0, 1, 32, length($png[$i]), $offset);
  $body .= $png[$i];
  $offset += length($png[$i]);
}
open(my $o, '>:raw', $out) or die "open $out: $!";
print $o $header . $entries . $body;
close $o;
print "wrote $out\n";
