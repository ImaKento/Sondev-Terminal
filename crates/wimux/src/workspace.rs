//! ワークスペース → タブ → 分割ツリー（ペイン）のモデル。
//! UI に依存しない部分だけを置き、テストできるようにしている。

use crate::session::Axis;

/// ターミナル 1 つ（= タブ 1 つ）の ID
pub type PaneId = usize;
/// 分割の区画の ID（Node の葉）
pub type GroupId = usize;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(PaneId),
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// 0〜1 の正規化座標での矩形
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Node {
    /// 左上から順にペインを列挙
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// `target` を分割して、その後ろ（右 or 下）に `new` を置く
    pub fn split(&mut self, target: PaneId, new: PaneId, axis: Axis) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split {
                    axis,
                    ratio: 0.5,
                    first: Box::new(Node::Leaf(target)),
                    second: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                first.split(target, new, axis) || second.split(target, new, axis)
            }
        }
    }

    /// `target` を分割して `new` を置く。`before` なら前（左 or 上）、そうでなければ後ろ（右 or 下）
    pub fn split_at(&mut self, target: PaneId, new: PaneId, axis: Axis, before: bool) -> bool {
        if !self.split(target, new, axis) {
            return false;
        }
        if before {
            self.swap_children_of(target, new);
        }
        true
    }

    /// `a` と `b` を子に持つ分割の、子の順番を入れ替える
    fn swap_children_of(&mut self, a: PaneId, b: PaneId) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                let pair = matches!((&**first, &**second), (Node::Leaf(x), Node::Leaf(y))
                    if (*x == a && *y == b) || (*x == b && *y == a));
                if pair {
                    std::mem::swap(first, second);
                    true
                } else {
                    first.swap_children_of(a, b) || second.swap_children_of(a, b)
                }
            }
        }
    }

    /// ペインを取り除く。ツリーが空になったら `None`
    pub fn remove(self, target: PaneId) -> Option<Node> {
        match self {
            Node::Leaf(id) if id == target => None,
            leaf @ Node::Leaf(_) => Some(leaf),
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.remove(target), second.remove(target)) {
                (Some(a), Some(b)) => Some(Node::Split {
                    axis,
                    ratio,
                    first: Box::new(a),
                    second: Box::new(b),
                }),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            },
        }
    }

    /// 各ペインの位置（0〜1 の正規化座標）
    pub fn rects(&self) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        self.collect_rects(
            Rect {
                x: 0.,
                y: 0.,
                w: 1.,
                h: 1.,
            },
            &mut out,
        );
        out
    }

    fn collect_rects(&self, r: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, r)),
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, b) = match axis {
                    Axis::Horizontal => (
                        Rect {
                            w: r.w * ratio,
                            ..r
                        },
                        Rect {
                            x: r.x + r.w * ratio,
                            w: r.w * (1. - ratio),
                            ..r
                        },
                    ),
                    Axis::Vertical => (
                        Rect {
                            h: r.h * ratio,
                            ..r
                        },
                        Rect {
                            y: r.y + r.h * ratio,
                            h: r.h * (1. - ratio),
                            ..r
                        },
                    ),
                };
                first.collect_rects(a, out);
                second.collect_rects(b, out);
            }
        }
    }

    /// `from` から見て `dir` 方向にある隣のペイン
    pub fn neighbor(&self, from: PaneId, dir: Direction) -> Option<PaneId> {
        const EPS: f32 = 1e-4;
        let rects = self.rects();
        let cur = rects.iter().find(|(id, _)| *id == from)?.1;
        let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| (a1.min(b1) - a0.max(b0)).max(0.);

        rects
            .iter()
            .filter(|(id, _)| *id != from)
            .filter_map(|(id, r)| {
                let (gap, ov) = match dir {
                    Direction::Left => (
                        cur.x - (r.x + r.w),
                        overlap(cur.y, cur.y + cur.h, r.y, r.y + r.h),
                    ),
                    Direction::Right => (
                        r.x - (cur.x + cur.w),
                        overlap(cur.y, cur.y + cur.h, r.y, r.y + r.h),
                    ),
                    Direction::Up => (
                        cur.y - (r.y + r.h),
                        overlap(cur.x, cur.x + cur.w, r.x, r.x + r.w),
                    ),
                    Direction::Down => (
                        r.y - (cur.y + cur.h),
                        overlap(cur.x, cur.x + cur.w, r.x, r.x + r.w),
                    ),
                };
                (gap > -EPS && ov > EPS).then_some((*id, gap, ov))
            })
            // 一番近く、重なりが一番大きいもの
            .min_by(|a, b| a.1.total_cmp(&b.1).then(b.2.total_cmp(&a.2)))
            .map(|(id, _, _)| id)
    }
}

/// 分割の 1 区画。中に複数のタブ（ターミナル）を持つ（cmux と同じ構成）
#[derive(Debug, Clone)]
pub struct Group {
    pub tabs: Vec<PaneId>,
    pub active: usize,
}

impl Group {
    pub fn new(pane: PaneId) -> Self {
        Self {
            tabs: vec![pane],
            active: 0,
        }
    }

    pub fn active_pane(&self) -> Option<PaneId> {
        self.tabs.get(self.active).copied()
    }

    /// タブを取り除く。空になったら true
    pub fn remove(&mut self, pane: PaneId) -> bool {
        if let Some(i) = self.tabs.iter().position(|p| *p == pane) {
            self.tabs.remove(i);
            if self.active > i || self.active >= self.tabs.len() {
                self.active = self.active.saturating_sub(1);
            }
        }
        self.tabs.is_empty()
    }
}

/// ワークスペース = 分割ツリー（葉は Group の ID）
#[derive(Debug, Clone)]
pub struct Workspace {
    /// ユーザーが付けた名前
    pub custom_name: Option<String>,
    pub root: Node,
    /// フォーカスしている区画
    pub focused: GroupId,
    /// 未読の通知数
    pub unread: usize,
    pub last_notification: Option<String>,
}

impl Workspace {
    pub fn new(group: GroupId) -> Self {
        Self {
            custom_name: None,
            root: Node::Leaf(group),
            focused: group,
            unread: 0,
            last_notification: None,
        }
    }

    pub fn groups(&self) -> Vec<GroupId> {
        self.root.leaves()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_before() {
        let mut root = Node::Leaf(1);
        assert!(root.split_at(1, 2, Axis::Horizontal, true));
        assert_eq!(root.leaves(), vec![2, 1]);
        assert!(root.split_at(1, 3, Axis::Vertical, false));
        assert_eq!(root.leaves(), vec![2, 1, 3]);
        assert!(root.split_at(1, 4, Axis::Vertical, true));
        assert_eq!(root.leaves(), vec![2, 4, 1, 3]);
    }

    #[test]
    fn split_and_neighbors() {
        // 1 | 2
        //   | 3
        let mut root = Node::Leaf(1);
        assert!(root.split(1, 2, Axis::Horizontal));
        assert!(root.split(2, 3, Axis::Vertical));
        assert_eq!(root.leaves(), vec![1, 2, 3]);
        assert_eq!(root.neighbor(1, Direction::Right), Some(2));
        assert_eq!(root.neighbor(3, Direction::Left), Some(1));
        assert_eq!(root.neighbor(2, Direction::Down), Some(3));
        assert_eq!(root.neighbor(3, Direction::Up), Some(2));
        assert_eq!(root.neighbor(1, Direction::Left), None);
    }

    #[test]
    fn group_remove_adjusts_active() {
        let mut g = Group::new(1);
        g.tabs.extend([2, 3]);
        g.active = 2;
        assert!(!g.remove(3));
        assert_eq!(g.active_pane(), Some(2));
        g.active = 1;
        assert!(!g.remove(1));
        assert_eq!(g.active_pane(), Some(2));
        assert!(g.remove(2));
    }

    #[test]
    fn remove_collapses_split() {
        let mut root = Node::Leaf(1);
        root.split(1, 2, Axis::Horizontal);
        root.split(2, 3, Axis::Vertical);
        let root = root.remove(2).unwrap();
        assert_eq!(root.leaves(), vec![1, 3]);
        let root = root.remove(1).unwrap();
        assert_eq!(root, Node::Leaf(3));
        assert!(root.remove(3).is_none());
    }
}
