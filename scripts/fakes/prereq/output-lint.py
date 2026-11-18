#!/usr/bin/env python3
"""F-7 r4: require checks on direct output commands in named shell bodies.

This deliberately conservative line lint is not a shell parser. It does not
inspect writes hidden in helpers, here-documents, redirects to files, command substitutions, dynamic
command names or unrecognized writer commands; it cannot prove a check
propagates the required status. Test those paths.
"""
import pathlib
import re
import sys

SCOPES = {
    'prereq_msg', 'machine_look_body', 'prereq_run_body',
    'prereq_requalify_compose_body', 'prereq_requalify_compose_inner_body',
}
WRITER = re.compile(r'(?<![\w:])(?:say|warn|prereq_msg|prereq_info|printf|echo|cat|awk|sed|tee|head|tail|grep)\s')
CHECK = re.compile(r'\|\|\s*(?:return|exit)\b')
START = re.compile(r'^([a-zA-Z_][a-zA-Z_0-9]*)\(\)\s*\{')


def mask(line):
    """Mask quoted content and comments, retaining shell control syntax."""
    out = []
    quote = None
    escaped = False
    parameter_depth = 0
    for position, ch in enumerate(line):
        if escaped:
            out.append(' ')
            escaped = False
        elif ch == '\\' and quote != "'":
            out.append(' ')
            escaped = True
        elif quote:
            out.append(' ')
            if ch == quote:
                quote = None
        elif ch in "'\"":
            quote = ch
            out.append(' ')
        elif ch == '{' and position and line[position - 1] == '$':
            parameter_depth += 1
            out.append(ch)
        elif ch == '}' and parameter_depth:
            parameter_depth -= 1
            out.append(ch)
        elif ch == '#' and not parameter_depth:
            out.extend(' ' * (len(line) - len(out)))
            break
        else:
            out.append(ch)
    return ''.join(out)


def audit(path, exceptions):
    current = None
    errors = []
    for n, line in enumerate(path.read_text().splitlines(), 1):
        start = START.match(line)
        if start:
            current = start.group(1) if start.group(1) in SCOPES else None
        if current is None:
            continue
        masked = mask(line)
        for match in WRITER.finditer(masked):
            # A command substitution's temporary pipe is not an external write;
            # its status still needs its own functional check.
            if '$(' in masked[:match.start()] and ')' not in masked[:match.start()].split('$(')[-1]:
                continue
            tail = masked[match.end():].split(';', 1)[0]
            # The reviewed form is a local || return/exit, not a distant check.
            key = f'{current}:{line.strip()}'
            approved = key in exceptions and 'best-effort' in line and re.search(r'\|\|\s*:', tail)
            check = CHECK.search(tail)
            pipeline = check and re.search(r'(?<!\|)\|(?!\|)', tail[:check.start()])
            if (not check or pipeline) and not approved:
                errors.append(f'{path}:{n}: {current}: unchecked {match.group().strip()}')
        if line == '}':
            current = None
    return errors


def main():
    if len(sys.argv) != 3:
        raise SystemExit('usage: output-lint.py INCLUDE EXCEPTION_LIST')
    path, exception_path = map(pathlib.Path, sys.argv[1:])
    exceptions = {line.strip() for line in exception_path.read_text().splitlines()
                  if line.strip() and not line.lstrip().startswith('#')}
    errors = audit(path, exceptions)
    if errors:
        print('\n'.join(errors), file=sys.stderr)
        return 1
    print('PASS F-7 r4: named direct output writes use checked form or listed exception')
    return 0


if __name__ == '__main__':
    sys.exit(main())
