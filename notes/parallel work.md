Here is the latest. Great job! it's now working in parallel, but our CPU usage isn't spiking and this feels like an even more parallelizable workload. I feel 


So it's working in parallel and it's using rayon.but we want to maximize the CPU usage.can do the work stealing stuff properly, which means we have to decomposesuch that things run in the sequence they have to, butit's not blocking each other so if there's a five step process and we have to apply that to each disk then we don't want to have each disk do step one in lock stepthe fastest ones to be able to race ahead inspread evenly.and one pattern that Ithink is very nice isWe have a struct that represents effectively a function call, and we implement the IntoFuture trait for that struct so that instead of just calling an asynchronous function, yourcreating the struct.and then you can have helper functions that just take in some arguments and transform them into thestruct and that isimportant because theInstead of being instance methods, they're just...parentless.vids that change the aesthetic of the cookie.to maximize the usage. I think instead of rayon we can use Tokyo.decompose the problem so that the flow of the data ispretty obvious from what the structs are, whereas when you have these function calls that are just chaining, there's noobvious pause for you to do whereas bycreating these.objects we can.kind of inspect what's going on.

D:\Repos\Azure\Cloud-Terrastodon\crates\azure\src\resource_groups.rs

There's an example

Please create a plan to convert our workload to this kind of pattern to maximizethe concurrent work being done.We want to saturate the CPU.