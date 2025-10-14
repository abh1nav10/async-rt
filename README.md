*--Introduction--*

This repository houses the implementation of a lock free stack and a lock free queue. When data structures are to be concurrently accessed by multiple threads, the most obvious and straightforward way to do this is
to wrap it inside a Mutex thus using coarse grained locking. This is a good choice for many usecases but the biggest challenge that makes such data structures vulnerable is preemptive scheduling of threads by the 
operating system. 

*--Problem in lock based data structures--*

Assume a thread currently holds the lock and gets preempted before releasing, then other threads will just wait for it to be rescheduled and release the lock before 
they can move forward which can even never happen. 

*--Solution--*

Lock free data structures solve this problem by promising that the system as a whole will never enter into a deadlock and some thread will make progress in a finite amount of time. This is done using low level 
atomic primitives like compare_and_exchange in rust. 

*--Memory Reclamation in Lock based data structures--*

A challenge that comes up in lock free data structures is safe memory reclamation. Taking the example of a queue, assume that a node has been dequeued but some other thread had loaded it prior to that and still holds
the raw pointer to the memory location. Deallocating the memory right after dequeuing is not safe as if that other thread dereferences the pointer aftwerwards, that would be a dangling pointer dereference and will
lead to the much dreaded undefined behaviour. 

*--Hazard Pointers--*

The way I have solved this is by implementing a well recognized solution known by the name of hazard pointers. Hazard Pointers maintain a list of all such pointers which 
are have been loaded by other threads. On any dequeue we retire the pointer thereby stating that the pointer is now in a state where the underlying memory should be deallocated as soon as we come to know that no other 
thread is currently holding it into a hazard pointer. The memory is thus safely reclaimed once there are no other holders thereby ensuring that no dangling pointer dereferences happen and the underlying memory is 
also safely reclaimed. 

*--ABA Problem--*

Another well known problem that comes while using compare_and_exchange instead of mutexes is the ABA problem. Before the exchange we load the atomic pointer and then try to compare and exchange it with something else.
This entire operation is not atomic and exposes a tiny window where things could go wrong. Assume that after the load some other thread deallocated the memory pointed to by that pointer and then a new allocation 
happens at the same address. The compare_and_exchange will still succeed but the structure of our data structure may get corrupted. 

*--Hazard Pointers as the solution for ABA--*

This problem is also solved by hazard pointers as in instead of directly loading the atomic pointer we load it into a hazard pointer. This will ensure that even if the atomic pointer gets swapped in between the load and the compare_and_exchange the memory that the pointer that we loaded was pointing to will not get deallocated as hazard pointer will prevent that from happening and thus no new allocation can happen at that same spot therefore preventing the ABA problem. The worst that could now happen for us is that our compare_and_exchange will fail but it will never lead to the data structure getting corrupted.

*--Testing--*

The implementation has been tested with loom which ensures that it is correct under all possible relevant thread interleavings. Further the data structures have also been benchmarked against their locking 
counterparts using std::sync::Mutex. 

*--Usage--*
The usage is fairly straightforward. It just involves wrapping the stack or the queue in an Arc and then passing it to other spawned threads. That ensures that when the drop implementation is run no other thread is enqueuing or dequeuing.

*--Vision--*

I will keep optimizing the data structure as I go on exploring the depths of multiprocessor programming. One such issue which appears more seriously in the stack implementation is of cache line ping ponging wherein 
multiple threads try to update the same head pointer thus leading to severe traffic under a highly concurrent scenario. This could lead to the system being underperformant under severe contention. One way this 
problem is solved is through ensuring proper alignment but since the a single location is getting contented it doesnt help. This could have helped if two locations being modified concurrently rest on the same
cache line. The contention could be reduced through a strategy more commonly used in lock based data structures known as exponential backoff. 
I will implement optimal solutions for these problems in the coming days.

