use std::{
    cmp::Reverse,
    collections::{
        hash_map::Entry::{Occupied, Vacant},
        BinaryHeap,
    },
    fmt::Debug,
    hash::Hash,
    iter::Sum,
    marker::PhantomData,
    ops::Add,
    ops::Sub,
    sync::Arc,
};

use fxhash::{FxHashMap, FxHashSet};
use parking_lot::Mutex;

use crate::{Heuristic, LimitValues, State, Task, TransitionSystem};

use super::SearchNode;

/// Implementation of the Reverse Resumable A* algorithm
/// that computes the shortest path between:
/// - any state of a given transition system, and
/// - the goal state of a given task in this transition system.
///
/// The shortest paths are computed on demand by the heuristic requests.
pub struct ReverseResumableAStar<TS, S, A, C, DC, H>
where
    TS: TransitionSystem<S, A, C, DC>,
    S: Debug + State + Hash + Eq + Clone,
    C: Eq
        + PartialOrd
        + Ord
        + Add<DC, Output = C>
        + Sub<C, Output = DC>
        + Copy
        + Default
        + LimitValues,
    DC: Copy,
    H: Heuristic<TS, S, A, C, DC>,
{
    transition_system: Arc<TS>,
    task: Arc<Task<S, C>>,
    /// The heuristic must be an estimate of the distance to the start state
    heuristic: H,
    data: Mutex<RraData<S, C, DC>>,
    _phantom: PhantomData<A>,
}

impl<TS, S, A, C, DC, H> Heuristic<TS, S, A, C, DC> for ReverseResumableAStar<TS, S, A, C, DC, H>
where
    TS: TransitionSystem<S, A, C, DC>,
    S: Debug + State + Hash + Eq + Clone,
    C: Eq
        + PartialOrd
        + Ord
        + Add<DC, Output = C>
        + Sub<C, Output = DC>
        + Copy
        + Default
        + LimitValues,
    DC: Copy,
    H: Heuristic<TS, S, A, C, DC>,
{
    fn get_heuristic(&self, state: &S) -> Option<DC> {
        self.find_path(state)
    }
}

impl<TS, S, A, C, DC, H> ReverseResumableAStar<TS, S, A, C, DC, H>
where
    TS: TransitionSystem<S, A, C, DC>,
    S: Debug + State + Hash + Eq + Clone,
    C: Eq
        + PartialOrd
        + Ord
        + Add<DC, Output = C>
        + Sub<C, Output = DC>
        + Copy
        + Default
        + LimitValues,
    DC: Copy,
    H: Heuristic<TS, S, A, C, DC>,
{
    /// Creates a new instance of the RRA* algorithm
    ///
    /// # Arguments
    ///
    /// * `transition_system` - The transition system in which the agents navigate.
    /// * `task` - The task to solve.
    /// * `heuristic` - The heuristic to use to guide the search.
    pub fn new(transition_system: Arc<TS>, task: Arc<Task<S, C>>, heuristic: H) -> Self
    where
        Self: Sized,
    {
        let mut rra = ReverseResumableAStar {
            transition_system: transition_system.clone(),
            task: task.clone(),
            heuristic,
            data: Mutex::new(RraData::default()),
            _phantom: PhantomData,
        };
        rra.init();
        rra
    }

    /// Initializes the reverse search algorithm by enqueueing the goal state.
    fn init(&mut self) {
        let goal_node = SearchNode {
            state: Arc::new(self.task.goal_state.clone()),
            cost: self.task.initial_cost,
            heuristic: C::default() - C::default(),
        };

        let mut data = self.data.lock();
        data.distance
            .insert(goal_node.state.clone(), goal_node.cost);
        data.queue.push(Reverse(goal_node));
    }

    /// Computes the shortest path between the given state and the goal state,
    /// or returns directly if it has already been computed.
    fn find_path(&self, state: &S) -> Option<DC> {
        let mut data = self.data.lock();

        if data.closed.contains(state) {
            // The distance has already been computed
            data.stats.cached_query += 1;
            return Some(data.distance[state] - self.task.initial_cost);
        }

        data.stats.new_query += 1;

        while let Some(Reverse(current)) = data.queue.pop() {
            if current.cost > data.distance[&current.state] {
                // A better path has already been found
                continue;
            }

            data.closed.insert(current.state.clone()); // Mark the state as closed because the optimal distance has been found

            if *current.state == *state {
                // The optimal distance has been found
                let cost = current.cost - self.task.initial_cost;
                // Re-insert the current node because it has not been expanded
                data.queue.push(Reverse(current));
                return Some(cost);
            }

            // Expand the current state and enqueue its successors if a better path has been found
            for action in self.transition_system.reverse_actions_from(&current.state) {
                let successor_state = Arc::new(
                    self.transition_system
                        .reverse_transition(&current.state, &action),
                );

                let successor_cost = current.cost
                    + self
                        .transition_system
                        .reverse_transition_cost(&current.state, &action);

                let improved = match data.distance.entry(successor_state.clone()) {
                    Occupied(mut e) => {
                        if successor_cost < *e.get() {
                            *e.get_mut() = successor_cost;
                            true
                        } else {
                            false
                        }
                    }
                    Vacant(e) => {
                        e.insert(successor_cost);
                        true
                    }
                };

                if improved {
                    if let Some(heuristic) = self.heuristic.get_heuristic(&successor_state) {
                        data.queue.push(Reverse(SearchNode {
                            state: successor_state,
                            cost: successor_cost,
                            heuristic,
                        }));
                    }
                }
            }

            data.stats.expanded += 1;
        }

        None
    }

    /// Returns the statistics of the search algorithm.
    pub fn get_stats(&self) -> RraStats {
        self.data.lock().stats
    }
}

/// Protected data used by the Reverse Resumable A* algorithm.
struct RraData<S, C, DC>
where
    C: Copy + Ord + Add<DC, Output = C>,
    DC: Copy,
{
    queue: BinaryHeap<Reverse<SearchNode<S, C, DC>>>,
    distance: FxHashMap<Arc<S>, C>,
    closed: FxHashSet<Arc<S>>,
    stats: RraStats,
}

impl<S, C, DC> Default for RraData<S, C, DC>
where
    C: Copy + Ord + Add<DC, Output = C>,
    DC: Copy,
{
    fn default() -> Self {
        Self {
            queue: BinaryHeap::new(),
            distance: FxHashMap::default(),
            closed: FxHashSet::default(),
            stats: RraStats::default(),
        }
    }
}

/// Statistics of the Reverse Resumable A* algorithm.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RraStats {
    /// The number of new queries.
    pub new_query: usize,
    /// The number of cached queries (for which the heuristic value has already been computed).
    pub cached_query: usize,
    /// The number of expanded search nodes.
    pub expanded: usize,
}

impl Sum for RraStats {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), |a, b| Self {
            new_query: a.new_query + b.new_query,
            cached_query: a.cached_query + b.cached_query,
            expanded: a.expanded + b.expanded,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ordered_float::OrderedFloat;

    use crate::{
        simple_graph, GraphNodeId, Heuristic, ReverseResumableAStar, RraStats, SimpleHeuristic,
        SimpleState, SimpleWorld, Task,
    };

    #[test]
    fn test_simple() {
        let size = 10;
        let graph = simple_graph(size);
        let transition_system = Arc::new(SimpleWorld::new(graph, 0.4));
        let task = Arc::new(Task::new(
            SimpleState(GraphNodeId(0)),
            SimpleState(GraphNodeId(size * size - 1)),
            OrderedFloat(0.0),
        ));
        let heuristic = ReverseResumableAStar::new(
            transition_system.clone(),
            task.clone(),
            SimpleHeuristic::new(transition_system, Arc::new(task.reverse())),
        );

        for x in 0..size {
            for y in 0..size {
                assert_eq!(
                    heuristic
                        .get_heuristic(&SimpleState(GraphNodeId(x + y * size)))
                        .unwrap(),
                    OrderedFloat(((size - x - 1) + (size - y - 1)) as f64)
                );
            }
        }
    }

    #[test]
    fn test_caching() {
        let size = 10;
        let graph = simple_graph(size);
        let transition_system = Arc::new(SimpleWorld::new(graph, 0.4));
        let task = Arc::new(Task::new(
            SimpleState(GraphNodeId(0)),
            SimpleState(GraphNodeId(size * size - 1)),
            OrderedFloat(0.0),
        ));
        let heuristic = ReverseResumableAStar::new(
            transition_system.clone(),
            task.clone(),
            SimpleHeuristic::new(transition_system, Arc::new(task.reverse())),
        );
        let initial = heuristic.get_stats();
        heuristic.get_heuristic(&SimpleState(GraphNodeId(0)));
        let after_one_query = heuristic.get_stats();
        heuristic.get_heuristic(&SimpleState(GraphNodeId(0)));
        let after_same_query = heuristic.get_stats();
        assert_eq!(
            initial,
            RraStats {
                new_query: 0,
                cached_query: 0,
                expanded: 0
            }
        );
        assert_eq!(after_one_query.new_query, 1);
        assert_eq!(after_one_query.cached_query, 0);
        assert_eq!(after_same_query.new_query, 1);
        assert_eq!(after_same_query.cached_query, 1);
        assert_eq!(after_same_query.expanded, after_one_query.expanded);
    }
}
