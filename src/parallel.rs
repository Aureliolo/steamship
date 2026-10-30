//! Work over many files spread across the machine's cores, as checking steamcmd's install on every
//! run hashes and asks Windows about each of its files.

use std::num::NonZeroUsize;
use std::panic;
use std::thread;

/// `each` of `items`, in their order, worked through on as many threads as the machine has cores.
pub fn map<Item, Out, Each>(items: &[Item], each: Each) -> Vec<Out>
where
    Item: Sync,
    Out: Send,
    Each: Fn(&Item) -> Out + Sync,
{
    let cores = thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let share = items.len().div_ceil(cores).max(1);
    thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks(share)
            .map(|chunk| scope.spawn(|| chunk.iter().map(&each).collect::<Vec<_>>()))
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|thrown| panic::resume_unwind(thrown))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_is_worked_in_its_order() {
        let items: Vec<u32> = (0..1000).collect();
        assert_eq!(
            map(&items, |item| item * 2),
            items.iter().map(|item| item * 2).collect::<Vec<_>>()
        );
        let none: [u32; 0] = [];
        assert_eq!(map(&none, |item| *item), Vec::<u32>::new());
    }
}
