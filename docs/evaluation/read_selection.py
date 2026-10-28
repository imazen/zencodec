#!/usr/bin/env python3
"""Read one frozen code-first site step; preserve links and collapsed context.

Unlike the original no-feedback learner protocol, this helper is used by readers
who preserve an attempt, compile it, expand context and then repair it. The input
is a captured rendered page, not a hand-written substitute for its examples.
"""
import argparse
from html.parser import HTMLParser
from pathlib import Path


class Node:
    def __init__(self, tag='', attrs=()):
        self.tag, self.attrs, self.children = tag, dict(attrs), []

    def text(self):
        return ''.join(c if isinstance(c, str) else c.text() for c in self.children)

    def find(self, predicate):
        if predicate(self):
            yield self
        for child in self.children:
            if isinstance(child, Node):
                yield from child.find(predicate)


class Page(HTMLParser):
    VOID = {'area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'param', 'source', 'track', 'wbr'}

    def __init__(self, html):
        super().__init__(convert_charrefs=True)
        self.root = Node()
        self.stack = [self.root]
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        node = Node(tag, attrs)
        self.stack[-1].children.append(node)
        if tag not in self.VOID:
            self.stack.append(node)

    def handle_endtag(self, tag):
        for index in range(len(self.stack) - 1, 0, -1):
            if self.stack[index].tag == tag:
                del self.stack[index:]
                break

    def handle_data(self, text):
        self.stack[-1].children.append(text)


def render(node, expanded):
    if isinstance(node, str):
        return node
    if node.tag in {'script', 'style'} or 'sr-only' in node.attrs.get('class', '').split():
        return ''
    if node.tag == 'pre':
        return '\n```rust\n' + node.text().strip('\n') + '\n```\n'
    if node.tag == 'button':
        return '\n[Button: ' + node.text() + ']\n'
    if node.tag == 'span':
        return ' ' + ''.join(render(child, expanded) for child in node.children) + ' '
    if node.tag == 'a':
        return '[' + node.text() + '](' + node.attrs.get('href', '') + ')'
    if node.tag == 'details' and not expanded and 'open' not in node.attrs:
        summary = next((c for c in node.children if isinstance(c, Node) and c.tag == 'summary'), None)
        return '\n[Collapsed: ' + (summary.text() if summary else 'details') + ']\n'
    body = ''.join(render(child, expanded) for child in node.children)
    if node.tag in {'p', 'div', 'aside', 'summary', 'h2', 'h3', 'nav', 'details'}:
        return '\n' + body + '\n'
    return body


def selection(html, flow_id, step_number, expanded):
    root = Page(html).root
    matches = list(root.find(lambda n: n.tag == 'article' and n.attrs.get('id') == flow_id))
    if len(matches) != 1:
        raise ValueError(f'Expected one flow: {flow_id}')
    flow = matches[0]
    steps = list(flow.find(lambda n: n.tag == 'details' and 'cf-step' in n.attrs.get('class', '').split()))
    if not 1 <= step_number <= len(steps):
        raise ValueError(f'{flow_id} has {len(steps)} steps')
    step = steps[step_number - 1]
    step.attrs['open'] = None  # Opening this step does not open its nested context.
    role = next(r for r in ('consumer', 'adapter', 'implementor') if r in flow.attrs.get('class', '').split())
    panel = next(root.find(lambda n: n.attrs.get('id') == 'role-' + role))
    story = next(panel.find(lambda n: 'cf-story' in n.attrs.get('class', '').split()))
    prefix = [story] + [n for n in flow.children if isinstance(n, Node) and n.tag in {'h2', 'aside'}]
    return ''.join(render(n, expanded) for n in prefix) + render(step, expanded)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('flow')
    parser.add_argument('step', type=int)
    parser.add_argument('view', choices=('visible', 'expanded'))
    parser.add_argument('--page', type=Path, default=Path(__file__).with_name('code.html'))
    args = parser.parse_args()
    print(f'# Site selection: {args.flow}, step {args.step}, {args.view}\n')
    print(selection(args.page.read_text(), args.flow, args.step, args.view == 'expanded'))


if __name__ == '__main__':
    main()
