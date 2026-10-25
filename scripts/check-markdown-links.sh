#!/bin/sh
# Check every relative link in the tracked Markdown files. A link target must
# be a tracked file or a directory that contains one; a #fragment must name a
# heading (also inside a blockquote) or an explicit HTML id of the target
# Markdown file. Heading anchors follow GitHub's rules: underscore emphasis
# removed, lower case, characters other than letters, digits, marks,
# underscores, hyphens and spaces dropped, spaces turned into hyphens, and -1,
# -2, ... appended to repeated anchors. External URLs, fenced code, inline code
# and HTML comments are skipped, and so are line-anchor fragments into
# non-Markdown files or ?plain=1 source views. Four-space indented code blocks
# are read as prose, so example links belong in fenced code. Prints one line
# per problem and fails if any exist.
set -eu
cd "$(git rev-parse --show-toplevel)"
exec perl -w - <<'PERL'
use strict;
use feature qw(unicode_strings);

my %tracked;
my %directory = ('' => 1);
{
    local $/ = "\0";
    open my $list, '-|', 'git', 'ls-files', '-z' or die "git ls-files: $!\n";
    while (my $path = <$list>) {
        chomp $path;
        $tracked{$path} = 1;
        my $parent = $path;
        $directory{$parent} = 1 while $parent =~ s{/[^/]*\z}{};
    }
    close $list or die "git ls-files failed\n";
}

# Characters of one tracked file, or undef when it is not valid UTF-8.
sub text_of {
    my ($path) = @_;
    open my $file, '<:raw', $path or die "$path: $!\n";
    local $/;
    my $bytes = <$file>;
    close $file;
    return utf8::decode($bytes) ? $bytes : undef;
}

# Calls $visit->(line number, line) for every line outside fenced code, with
# HTML comments, including ones that span lines, removed.
sub prose_lines {
    my ($text, $visit) = @_;
    my ($fence, $comment, $number) = (undef, 0, 0);
    for my $line (split /\n/, $text) {
        $number++;
        $line =~ s/\r\z//;
        if (defined $fence) {
            undef $fence if $line =~ /^[ \t]*\Q$fence\E+[ \t]*$/;
            next;
        }
        if ($comment) {
            next unless $line =~ s/^.*?-->//;
            $comment = 0;
        } elsif ($line =~ /^[ \t]*(`{3,}|~{3,})/) {
            $fence = $1;
            next;
        }
        $line =~ s/<!--.*?-->//g;
        $comment = 1 if $line =~ s/<!--.*//;
        $visit->($number, $line);
    }
}

sub without_code_spans {
    my ($line) = @_;
    $line =~ s/(`+).+?(?<!`)\1(?!`)//g;
    return $line;
}

sub github_slug {
    my ($heading) = @_;
    $heading =~ s/!?\[([^\]]*)\]\([^)]*\)/$1/g;
    $heading =~ s/<[^>]*>//g;
    # GitHub slugs the rendered text: an underscore run that opens or closes
    # emphasis disappears, one inside a word or a code span stays.
    $heading =~ s{((`+).+?(?<!`)\2(?!`))|(?<!\w)_+(?!_)|(?<!_)_+(?!\w)}{$1 // ''}ge;
    $heading = lc $heading;
    $heading =~ s/[^\w\- ]//g;
    $heading =~ tr/ /-/;
    return $heading;
}

my %anchors_of;
sub anchors {
    my ($path) = @_;
    return $anchors_of{$path} if $anchors_of{$path};
    my (%anchor, %seen);
    my ($previous, $previous_number) = ('', 0);
    my $add_heading = sub {
        my ($heading) = @_;
        $heading =~ s/^\s+|\s+$//g;
        my $base = github_slug($heading);
        my $slug = $base;
        while (exists $seen{$slug}) {
            $seen{$base}++;
            $slug = "$base-$seen{$base}";
        }
        $seen{$slug} = 0;
        $anchor{$slug} = 1;
    };
    my $text = text_of($path);
    prose_lines($text // '', sub {
        my ($number, $line) = @_;
        # A setext underline belongs only to the line right above it, never to
        # one before a skipped fence or comment.
        $previous = '' if $number != $previous_number + 1;
        if ($line =~ /^(?: {0,3}>[ \t]?)* {0,3}#{1,6}(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/) {
            $add_heading->($1 // '');
        } elsif ($line =~ /^ {0,3}(?:=+|-+)[ \t]*$/ && $previous =~ /\S/
                 && $previous !~ /^\s*(?:[-*+>|#]|\d+[.)])/) {
            $add_heading->($previous);
        }
        $anchor{$1} = 1 while $line =~ /<[^>]*\b(?:id|name)=(?|"([^"]+)"|'([^']+)')/g;
        ($previous, $previous_number) = ($line, $number);
    });
    return $anchors_of{$path} = \%anchor;
}

# Repository-relative path, or undef when it leaves the repository.
sub resolve {
    my ($from, $target) = @_;
    my @parts;
    if ($target !~ m{^/}) {
        @parts = split m{/}, $from;
        pop @parts;
    }
    for my $part (split m{/}, $target) {
        next if $part eq '' || $part eq '.';
        if ($part eq '..') {
            return undef unless @parts;
            pop @parts;
        } else {
            push @parts, $part;
        }
    }
    return join '/', @parts;
}

sub percent_decoded {
    my ($value) = @_;
    utf8::encode($value);
    $value =~ s/%([0-9A-Fa-f]{2})/chr hex $1/ge;
    return $value;
}

my $problems = 0;
sub problem {
    my ($path, $number, $message) = @_;
    utf8::encode($message);
    print "$path:$number: $message\n";
    $problems++;
}

sub check_target {
    my ($path, $number, $target) = @_;
    $target =~ s/^<(.*)>$/$1/;
    return if $target =~ m{^(?:[A-Za-z][A-Za-z0-9+.-]*:|//)};
    my ($location, $fragment) = split /#/, $target, 2;
    # ?plain=1 shows Markdown source, where fragments are line anchors.
    my $source_view = $location =~ /\?(?:[^#]*&)?plain=1(?:&|\z)/;
    $location =~ s/\?.*//s;
    my $file = $path;
    if ($location ne '') {
        $file = resolve($path, percent_decoded($location));
        if (!defined $file) {
            problem($path, $number, "link leaves the repository: $target");
            return;
        }
        if (!$tracked{$file} && !$directory{$file}) {
            problem($path, $number, "link target is not tracked: $target");
            return;
        }
    }
    return if !defined $fragment || $fragment eq '' || $source_view
        || $file !~ /\.md\z/i || !$tracked{$file};
    my $name = percent_decoded($fragment);
    utf8::decode($name);
    problem($path, $number, "no heading or id for #$name in $file")
        unless anchors($file)->{$name};
}

{
    local $/ = "\0";
    open my $list, '-|', 'git', 'ls-files', '-z', '--', '*.md' or die "git ls-files: $!\n";
    while (my $path = <$list>) {
        chomp $path;
        my $text = text_of($path);
        if (!defined $text) {
            problem($path, 0, 'not valid UTF-8');
            next;
        }
        prose_lines($text, sub {
            my ($number, $line) = @_;
            $line = without_code_spans($line);
            check_target($path, $number, $1)
                while $line =~ /\]\(\s*(<[^>]*>|[^\s)]+)/g;
            check_target($path, $number, $1)
                if $line =~ /^ {0,3}\[(?!\^)[^\]]+\]:\s*(<[^>]*>|\S+)/;
            check_target($path, $number, $1)
                while $line =~ /\b(?:href|src)=(?|"([^"]*)"|'([^']*)')/g;
        });
    }
    close $list or die "git ls-files failed\n";
}

print "$problems Markdown link problem(s)\n";
exit($problems ? 1 : 0);
PERL
