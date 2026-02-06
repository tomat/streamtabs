#!/usr/bin/env perl
use strict;
use warnings;
use utf8;
use Encode qw(decode);

my $DEFAULT_FG = '#d9e1eb';
my $DEFAULT_BG = '#0d1117';

my $FONT_FAMILY = $ENV{ANSI_SVG_FONT_FAMILY} // 'Menlo-Regular';
my $FONT_SIZE = $ENV{ANSI_SVG_FONT_SIZE} // 28;
my $CHAR_WIDTH = $ENV{ANSI_SVG_CHAR_WIDTH} // 17.05;
my $LINE_HEIGHT = $ENV{ANSI_SVG_LINE_HEIGHT} // 40;
my $PAD_X = $ENV{ANSI_SVG_PAD_X} // 18;
my $PAD_Y = $ENV{ANSI_SVG_PAD_Y} // 16;
my $COLS = $ENV{ANSI_SVG_COLS};

sub usage {
    die "Usage: $0 <input-ansi.txt> <output.svg>\n";
}

sub xml_escape {
    my ($s) = @_;
    $s =~ s/&/&amp;/g;
    $s =~ s/</&lt;/g;
    $s =~ s/>/&gt;/g;
    $s =~ s/"/&quot;/g;
    $s =~ s/'/&apos;/g;
    return $s;
}

sub basic_color {
    my ($idx, $bright) = @_;
    my @normal = (
        '#000000', '#cd3131', '#0dbc79', '#e5e510',
        '#2472c8', '#bc3fbc', '#11a8cd', '#e5e5e5',
    );
    my @bright_colors = (
        '#666666', '#f14c4c', '#23d18b', '#f5f543',
        '#3b8eea', '#d670d6', '#29b8db', '#ffffff',
    );
    return $bright ? $bright_colors[$idx] : $normal[$idx];
}

sub xterm256_to_hex {
    my ($n) = @_;
    $n = int($n);
    $n = 0 if $n < 0;
    $n = 255 if $n > 255;

    if ($n < 16) {
        my @base = (
            '#000000', '#800000', '#008000', '#808000',
            '#000080', '#800080', '#008080', '#c0c0c0',
            '#808080', '#ff0000', '#00ff00', '#ffff00',
            '#0000ff', '#ff00ff', '#00ffff', '#ffffff',
        );
        return $base[$n];
    }

    if ($n >= 16 && $n <= 231) {
        my $v = $n - 16;
        my $r = int($v / 36);
        my $g = int(($v % 36) / 6);
        my $b = $v % 6;
        my @steps = (0, 95, 135, 175, 215, 255);
        return sprintf('#%02x%02x%02x', $steps[$r], $steps[$g], $steps[$b]);
    }

    my $gray = 8 + ($n - 232) * 10;
    return sprintf('#%02x%02x%02x', $gray, $gray, $gray);
}

sub apply_sgr {
    my ($params, $state) = @_;
    my @codes = length($params) ? split(/;/, $params) : (0);

    for (my $i = 0; $i < @codes; $i++) {
        my $c = $codes[$i];
        $c = 0 if !defined($c) || $c eq '';
        $c = int($c);

        if ($c == 0) {
            $state->{fg} = $DEFAULT_FG;
            $state->{bg} = undef;
            next;
        }
        if ($c == 39) {
            $state->{fg} = $DEFAULT_FG;
            next;
        }
        if ($c == 49) {
            $state->{bg} = undef;
            next;
        }
        if ($c >= 30 && $c <= 37) {
            $state->{fg} = basic_color($c - 30, 0);
            next;
        }
        if ($c >= 90 && $c <= 97) {
            $state->{fg} = basic_color($c - 90, 1);
            next;
        }
        if ($c >= 40 && $c <= 47) {
            $state->{bg} = basic_color($c - 40, 0);
            next;
        }
        if ($c >= 100 && $c <= 107) {
            $state->{bg} = basic_color($c - 100, 1);
            next;
        }
        if ($c == 38 || $c == 48) {
            my $target = $c == 38 ? 'fg' : 'bg';
            my $mode = $codes[$i + 1];
            $mode = '' if !defined($mode);

            if ($mode eq '5') {
                my $n = $codes[$i + 2];
                if (defined($n) && $n =~ /^\d+$/) {
                    $state->{$target} = xterm256_to_hex($n);
                }
                $i += 2;
                next;
            }

            if ($mode eq '2') {
                my ($r, $g, $b) = @codes[$i + 2, $i + 3, $i + 4];
                if (
                    defined($r) && defined($g) && defined($b)
                    && $r =~ /^\d+$/ && $g =~ /^\d+$/ && $b =~ /^\d+$/
                ) {
                    $r = 0 if $r < 0; $r = 255 if $r > 255;
                    $g = 0 if $g < 0; $g = 255 if $g > 255;
                    $b = 0 if $b < 0; $b = 255 if $b > 255;
                    $state->{$target} = sprintf('#%02x%02x%02x', $r, $g, $b);
                }
                $i += 4;
                next;
            }
        }
    }
}

sub push_segment_char {
    my ($segments, $state, $ch) = @_;
    if (@$segments
        && $segments->[-1]{fg} eq $state->{fg}
        && ((defined $segments->[-1]{bg} && defined $state->{bg} && $segments->[-1]{bg} eq $state->{bg})
            || (!defined $segments->[-1]{bg} && !defined $state->{bg}))
    ) {
        $segments->[-1]{text} .= $ch;
    } else {
        push @$segments, {
            text => $ch,
            fg => $state->{fg},
            bg => $state->{bg},
        };
    }
}

sub parse_line {
    my ($line) = @_;
    my $state = { fg => $DEFAULT_FG, bg => undef };
    my @segments;

    while (length($line)) {
        if ($line =~ s/^\e\[([0-9;]*)m//) {
            apply_sgr($1, $state);
            next;
        }
        if ($line =~ s/^\e\[[0-9;?]*[A-Za-z]//) {
            next;
        }
        if ($line =~ s/^\e.//) {
            next;
        }

        my $ch = substr($line, 0, 1, '');
        push_segment_char(\@segments, $state, $ch);
    }

    return \@segments;
}

my ($input, $output) = @ARGV;
usage() if !defined($input) || !defined($output);

open my $fh, '<:raw', $input or die "Failed to read $input: $!\n";
local $/;
my $raw = <$fh>;
close $fh;

my $content = decode('UTF-8', $raw);
my @lines = split(/\n/, $content, -1);
if (@lines && $lines[-1] eq '') {
    pop @lines;
}
for my $line (@lines) {
    $line =~ s/\r$//;
}

my @parsed;
my $max_chars = 1;
for my $line (@lines) {
    my $segments = parse_line($line);
    my $chars = 0;
    for my $seg (@$segments) {
        $chars += length($seg->{text});
    }
    $max_chars = $chars if $chars > $max_chars;
    push @parsed, $segments;
}

my $render_chars = $max_chars;
if (defined($COLS) && $COLS =~ /^\d+$/ && $COLS > 0) {
    $render_chars = $COLS;
}

my $width = int($PAD_X * 2 + $render_chars * $CHAR_WIDTH + 0.5);
my $height = int($PAD_Y * 2 + scalar(@parsed) * $LINE_HEIGHT + 0.5);
$height = int($PAD_Y * 2 + $LINE_HEIGHT + 0.5) if $height < int($PAD_Y * 2 + $LINE_HEIGHT + 0.5);

open my $out, '>:encoding(UTF-8)', $output or die "Failed to write $output: $!\n";
print {$out} qq{<svg xmlns="http://www.w3.org/2000/svg" width="$width" height="$height" viewBox="0 0 $width $height">\n};
print {$out} qq{  <rect width="100%" height="100%" fill="$DEFAULT_BG" />\n};
my $font = xml_escape($FONT_FAMILY);

for (my $row = 0; $row < @parsed; $row++) {
    my $segments = $parsed[$row];
    my $x = $PAD_X;
    my $y = $PAD_Y + ($row + 1) * $LINE_HEIGHT - ($LINE_HEIGHT - $FONT_SIZE) / 2.2;
    for my $seg (@$segments) {
        my $text = $seg->{text};
        next if $text eq '';

        my $fg = $seg->{fg} // $DEFAULT_FG;
        my $escaped = xml_escape($text);
        my $chars = length($text);
        my $w = $chars * $CHAR_WIDTH;

        if (defined $seg->{bg}) {
            my $rh = $LINE_HEIGHT;
            my $ry = $y - $FONT_SIZE + ($LINE_HEIGHT - $FONT_SIZE) / 3;
            print {$out} qq{  <rect x="$x" y="$ry" width="$w" height="$rh" fill="$seg->{bg}" />\n};
        }

        print {$out}
          qq{  <text x="$x" y="$y" fill="$fg" font-family="$font" font-size="$FONT_SIZE" font-style="normal" font-weight="400" xml:space="preserve">$escaped</text>\n};
        $x += $w;
    }
}

print {$out} "</svg>\n";
close $out;
