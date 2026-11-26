import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Search,
  Trash2,
  UserMinus,
  UserPlus,
  Users,
  X,
} from "lucide-react";
import { toast } from "sonner";
import type { Customer, CustomerGroup } from "@fastcart/shared-types";

import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, TableSkeleton } from "@/components/states";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { errorMessage } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import {
  addCustomerToGroup,
  createCustomerGroup,
  customerDisplayName,
  listCustomerGroups,
  removeCustomerFromGroup,
  searchCustomers,
  sortCustomerGroups,
} from "@/lib/customer-groups";
import { demoCustomers } from "@/lib/demo";
import { fromUnixSeconds, initials, shortDate } from "@/lib/format";
import { useAsyncData } from "@/lib/use-async-data";

/** A customer reduced to what the member picker needs. */
interface MemberCandidate {
  id: string;
  name: string;
  email: string;
}

/** Demo groups with their member ids; the API has no member listing route. */
const DEMO_GROUP_FIXTURES: Array<{ group: CustomerGroup; memberIds: string[] }> = [
  {
    group: {
      id: "cgroup_demo_vip",
      name: "VIP customers",
      metadata: {},
      created_at: 1_759_800_000,
      updated_at: 1_759_886_400,
    },
    memberIds: [],
  },
  {
    group: {
      id: "cgroup_demo_wholesale",
      name: "Wholesale",
      metadata: {},
      created_at: 1_760_320_000,
      updated_at: 1_760_406_400,
    },
    memberIds: [],
  },
  {
    group: {
      id: "cgroup_demo_lapsed",
      name: "Lapsed buyers",
      metadata: {},
      created_at: 1_761_000_000,
      updated_at: 1_761_086_400,
    },
    memberIds: [],
  },
];

function initialDemoGroups(): Array<{ group: CustomerGroup; memberIds: string[] }> {
  const customers = demoCustomers().customers;
  const first = customers[0];
  const second = customers[1];
  return DEMO_GROUP_FIXTURES.map((entry, index) => ({
    ...entry,
    memberIds: index === 0 ? [first?.id, second?.id].filter((id): id is string => Boolean(id)) : [],
  }));
}

function demoCandidates(): MemberCandidate[] {
  return demoCustomers().customers.map((customer) => ({
    id: customer.id,
    name: customer.name,
    email: customer.email,
  }));
}

function candidateFromCustomer(customer: Customer): MemberCandidate {
  return {
    id: customer.id,
    name: customerDisplayName(customer),
    email: customer.email,
  };
}

export function CustomerGroupsPage() {
  const { mode, store, dataKey, can } = useAuth();
  const [demoGroups, setDemoGroups] = useState(initialDemoGroups);
  const [search, setSearch] = useState("");

  const [createOpen, setCreateOpen] = useState(false);
  const [createBusy, setCreateBusy] = useState(false);
  const [newName, setNewName] = useState("");
  const [nameError, setNameError] = useState<string | null>(null);

  const [renameTarget, setRenameTarget] = useState<CustomerGroup | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [renameBusy, setRenameBusy] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<CustomerGroup | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);

  const [membersGroup, setMembersGroup] = useState<CustomerGroup | null>(null);
  const [memberQuery, setMemberQuery] = useState("");
  const [memberResults, setMemberResults] = useState<MemberCandidate[]>([]);
  const [memberSearching, setMemberSearching] = useState(false);
  const [memberBusyId, setMemberBusyId] = useState<string | null>(null);
  const [removeId, setRemoveId] = useState("");
  const [removeBusy, setRemoveBusy] = useState(false);
  /** Members added during this page session, keyed by group id. */
  const [sessionMembers, setSessionMembers] = useState<Record<string, MemberCandidate[]>>({});

  useEffect(() => {
    setCreateOpen(false);
    setRenameTarget(null);
    setDeleteTarget(null);
    setMembersGroup(null);
    setSessionMembers({});
    setSearch("");
  }, [store?.id, dataKey]);

  const loader = useCallback(
    (signal: AbortSignal): Promise<CustomerGroup[]> => {
      if (mode === "demo") {
        return Promise.resolve(sortCustomerGroups(demoGroups.map((entry) => entry.group)));
      }
      if (!store) return Promise.reject(new Error("No store is selected."));
      return listCustomerGroups(store.id, signal);
    },
    [mode, store, demoGroups],
  );

  const { data, error, loading, reload } = useAsyncData(loader, [
    mode,
    store?.id,
    dataKey,
    demoGroups,
  ]);

  const groups = useMemo(() => data ?? [], [data]);
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (needle.length === 0) return groups;
    return groups.filter((group) => group.name.toLowerCase().includes(needle));
  }, [groups, search]);

  const candidates = useMemo(() => (mode === "demo" ? demoCandidates() : []), [mode]);

  useEffect(() => {
    if (!membersGroup || mode !== "api" || !store) return;
    const controller = new AbortController();
    const handle = window.setTimeout(() => {
      setMemberSearching(true);
      searchCustomers(store.id, memberQuery, controller.signal)
        .then((customers) => {
          setMemberResults(customers.map(candidateFromCustomer));
        })
        .catch((cause: unknown) => {
          if (!controller.signal.aborted) toast.error(errorMessage(cause));
        })
        .finally(() => {
          if (!controller.signal.aborted) setMemberSearching(false);
        });
    }, 250);
    return () => {
      window.clearTimeout(handle);
      controller.abort();
      setMemberSearching(false);
    };
  }, [membersGroup, mode, store, memberQuery]);

  const demoMemberIds = useCallback(
    (groupId: string): string[] =>
      demoGroups.find((entry) => entry.group.id === groupId)?.memberIds ?? [],
    [demoGroups],
  );

  const sessionMembersFor = useCallback(
    (groupId: string): MemberCandidate[] => sessionMembers[groupId] ?? [],
    [sessionMembers],
  );

  const memberCount = (group: CustomerGroup): number | null => {
    if (mode === "demo") return demoMemberIds(group.id).length;
    // The API exposes group membership per customer, not per group, so a
    // reliable count is unavailable; the UI shows a dash instead of guessing.
    return null;
  };

  const suggestedMembers = useMemo(() => {
    const list = mode === "demo" ? candidates : memberResults;
    const needle = memberQuery.trim().toLowerCase();
    if (needle.length === 0) return list;
    return list.filter(
      (candidate) =>
        candidate.name.toLowerCase().includes(needle) ||
        candidate.email.toLowerCase().includes(needle),
    );
  }, [mode, candidates, memberResults, memberQuery]);

  const openMembers = (group: CustomerGroup) => {
    setMembersGroup(group);
    setMemberQuery("");
    setMemberResults([]);
    setRemoveId("");
  };

  const closeMembers = () => {
    setMembersGroup(null);
    setMemberQuery("");
    setMemberResults([]);
    setRemoveId("");
  };

  const addMember = async (candidate: MemberCandidate) => {
    if (!membersGroup || memberBusyId) return;
    const groupId = membersGroup.id;
    setMemberBusyId(candidate.id);
    try {
      if (mode === "demo") {
        const existing = demoMemberIds(groupId);
        if (existing.includes(candidate.id)) {
          toast.message(`${candidate.name} is already in this group`);
        } else {
          setDemoGroups((current) =>
            current.map((entry) =>
              entry.group.id === groupId
                ? { ...entry, memberIds: [...entry.memberIds, candidate.id] }
                : entry,
            ),
          );
          toast.success(`${candidate.name} added`);
        }
      } else {
        if (!store) throw new Error("No store is selected.");
        await addCustomerToGroup(store.id, groupId, candidate.id);
        toast.success(`${candidate.name} added to the group`);
      }
      setSessionMembers((current) => {
        const existing = current[groupId] ?? [];
        if (existing.some((entry) => entry.id === candidate.id)) return current;
        return { ...current, [groupId]: [...existing, candidate] };
      });
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setMemberBusyId(null);
    }
  };

  const removeMemberById = async (customerId: string, label?: string) => {
    if (!membersGroup) return;
    const cleaned = customerId.trim();
    if (cleaned.length === 0) {
      toast.error("Enter the customer id to remove.");
      return;
    }
    setRemoveBusy(true);
    const groupId = membersGroup.id;
    try {
      if (mode === "demo") {
        if (!demoMemberIds(groupId).includes(cleaned)) {
          toast.error("That customer is not in this group.");
          return;
        }
        setDemoGroups((current) =>
          current.map((entry) =>
            entry.group.id === groupId
              ? { ...entry, memberIds: entry.memberIds.filter((id) => id !== cleaned) }
              : entry,
          ),
        );
      } else {
        if (!store) throw new Error("No store is selected.");
        await removeCustomerFromGroup(store.id, groupId, cleaned);
      }
      setSessionMembers((current) => {
        const existing = current[groupId] ?? [];
        return {
          ...current,
          [groupId]: existing.filter((entry) => entry.id !== cleaned),
        };
      });
      setRemoveId("");
      toast.success(label ? `${label} removed from the group` : "Customer removed from the group");
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setRemoveBusy(false);
    }
  };

  const createGroup = async () => {
    if (createBusy) return;
    const name = newName.trim();
    if (name.length === 0) {
      setNameError("Give the group a name.");
      return;
    }
    setCreateBusy(true);
    try {
      if (mode === "demo") {
        const now = Math.floor(Date.now() / 1000);
        setDemoGroups((current) => [
          ...current,
          {
            group: {
              id: `cgroup_demo_${Date.now()}`,
              name,
              metadata: {},
              created_at: now,
              updated_at: now,
            },
            memberIds: [],
          },
        ]);
      } else {
        if (!store) throw new Error("No store is selected.");
        await createCustomerGroup(store.id, { name });
      }
      toast.success("Customer group created");
      setNewName("");
      setCreateOpen(false);
      reload();
    } catch (cause) {
      toast.error(errorMessage(cause));
    } finally {
      setCreateBusy(false);
    }
  };

  const renameGroup = async () => {
    if (!renameTarget || renameBusy) return;
    const name = renameValue.trim();
    if (name.length === 0) {
      toast.error("Give the group a name.");
      return;
    }
    setRenameBusy(true);
    try {
      setDemoGroups((current) =>
        current.map((entry) =>
          entry.group.id === renameTarget.id
            ? {
                ...entry,
                group: {
                  ...entry.group,
                  name,
                  updated_at: Math.floor(Date.now() / 1000),
                },
              }
            : entry,
        ),
      );
      setRenameTarget(null);
      toast.success("Customer group renamed in the demo");
    } finally {
      setRenameBusy(false);
    }
  };

  const deleteGroup = async () => {
    if (!deleteTarget || deleteBusy) return;
    setDeleteBusy(true);
    try {
      setDemoGroups((current) => current.filter((entry) => entry.group.id !== deleteTarget.id));
      if (membersGroup?.id === deleteTarget.id) closeMembers();
      setDeleteTarget(null);
      toast.success("Customer group deleted in the demo");
    } finally {
      setDeleteBusy(false);
    }
  };

  if (!data && loading) return <TableSkeleton />;
  if (!data && error) return <ErrorState message={error} onRetry={reload} />;
  if (!data) return null;

  const writable = can("customers.write");
  const activeSessionMembers = membersGroup ? sessionMembersFor(membersGroup.id) : [];

  return (
    <div className="space-y-6 animate-enter">
      <PageHeader
        title="Customer groups"
        description="Segment customers for pricing and promotions. Groups are store-scoped and unique by name."
        actions={
          writable ? (
            <Button
              onClick={() => {
                setNewName("");
                setNameError(null);
                setCreateOpen(true);
              }}
            >
              <Plus className="size-4" aria-hidden="true" />
              New group
            </Button>
          ) : null
        }
      />

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative w-full sm:w-[320px]">
          <Search
            className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
            aria-hidden="true"
          />
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search groups"
            className="pr-9 pl-9"
            aria-label="Search customer groups"
          />
          {search.length > 0 ? (
            <button
              type="button"
              onClick={() => setSearch("")}
              className="absolute top-1/2 right-2.5 flex size-6 -translate-y-1/2 cursor-pointer items-center justify-center rounded-full text-faint hover:bg-accent hover:text-foreground"
              aria-label="Clear search"
            >
              <X className="size-3.5" />
            </button>
          ) : null}
        </div>
        <p className="text-[11.5px] text-muted-foreground">
          {groups.length} {groups.length === 1 ? "group" : "groups"}
          {mode === "demo" ? " · sample data" : ""}
        </p>
      </div>

      <Card className="overflow-hidden">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="pl-6">Group</TableHead>
              <TableHead className="w-[160px]">Members</TableHead>
              <TableHead className="hidden w-[160px] md:table-cell">Created</TableHead>
              <TableHead className="hidden w-[160px] lg:table-cell">Updated</TableHead>
              <TableHead className="w-10 pr-5" aria-label="Actions" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {filtered.map((group) => {
              const count = memberCount(group);
              const added = sessionMembersFor(group.id).length;
              return (
                <TableRow key={group.id} className="h-[58px]">
                  <TableCell className="pl-6">
                    <div className="flex items-center gap-3">
                      <span className="flex size-8 items-center justify-center rounded-full bg-brand-soft text-brand-ink">
                        <Users className="size-4" aria-hidden="true" />
                      </span>
                      <div className="min-w-0">
                        <p className="truncate text-[12.5px] font-medium text-foreground">
                          {group.name}
                        </p>
                        <p className="mt-0.5 truncate font-mono text-[10px] text-faint">
                          {group.id}
                        </p>
                      </div>
                    </div>
                  </TableCell>
                  <TableCell className="text-[12px] text-ink-soft tnum">
                    {count === null ? (
                      <span
                        className="text-faint"
                        title="The API exposes group membership per customer, not per group."
                      >
                        —
                        {added > 0 ? (
                          <span className="ml-1.5 text-[10.5px] text-muted-foreground">
                            {added} added
                          </span>
                        ) : null}
                      </span>
                    ) : (
                      <span>
                        {count.toLocaleString("en-US")}{" "}
                        <span className="text-muted-foreground">
                          {count === 1 ? "customer" : "customers"}
                        </span>
                      </span>
                    )}
                  </TableCell>
                  <TableCell className="hidden text-[11.5px] text-muted-foreground md:table-cell">
                    {shortDate(fromUnixSeconds(group.created_at))}
                  </TableCell>
                  <TableCell className="hidden text-[11.5px] text-muted-foreground lg:table-cell">
                    {shortDate(fromUnixSeconds(group.updated_at))}
                  </TableCell>
                  <TableCell className="pr-5 text-right">
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={`Actions for ${group.name}`}
                          className="text-faint data-[state=open]:bg-accent"
                        >
                          <MoreHorizontal className="size-4" />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-[220px]">
                        <DropdownMenuLabel className="truncate">{group.name}</DropdownMenuLabel>
                        <DropdownMenuItem disabled={!writable} onSelect={() => openMembers(group)}>
                          <Users aria-hidden="true" />
                          Manage members
                        </DropdownMenuItem>
                        {mode === "demo" ? (
                          <>
                            <DropdownMenuSeparator />
                            <DropdownMenuItem
                              onSelect={() => {
                                setRenameValue(group.name);
                                setRenameTarget(group);
                              }}
                            >
                              <Pencil aria-hidden="true" />
                              Rename group
                            </DropdownMenuItem>
                            <DropdownMenuItem
                              variant="destructive"
                              onSelect={() => setDeleteTarget(group)}
                            >
                              <Trash2 aria-hidden="true" />
                              Delete group
                            </DropdownMenuItem>
                          </>
                        ) : (
                          <>
                            <DropdownMenuSeparator />
                            <DropdownMenuLabel className="max-w-[200px] text-[10.5px] leading-relaxed font-normal whitespace-normal text-faint">
                              This API build does not expose group rename or delete routes.
                            </DropdownMenuLabel>
                          </>
                        )}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </TableCell>
                </TableRow>
              );
            })}
            {filtered.length === 0 ? (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={5} className="p-0">
                  <EmptyState
                    title={groups.length === 0 ? "No customer groups yet" : "No matching groups"}
                    description={
                      groups.length === 0
                        ? "Create a group to target pricing and promotions at a segment of customers."
                        : "Try a different name."
                    }
                    action={
                      groups.length === 0 && writable ? (
                        <Button onClick={() => setCreateOpen(true)}>
                          <Plus className="size-4" aria-hidden="true" />
                          New group
                        </Button>
                      ) : null
                    }
                  />
                </TableCell>
              </TableRow>
            ) : null}
          </TableBody>
        </Table>
      </Card>

      <CreateGroupDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        name={newName}
        error={nameError}
        busy={createBusy}
        onNameChange={(value) => {
          setNewName(value);
          setNameError(null);
        }}
        onSubmit={() => void createGroup()}
      />

      <Dialog
        open={renameTarget !== null}
        onOpenChange={(open) => {
          if (!open && !renameBusy) setRenameTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Rename group</DialogTitle>
            <DialogDescription>
              Demo mode keeps the change locally; the live API has no rename route yet.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="group-rename">Name</Label>
            <Input
              id="group-rename"
              value={renameValue}
              onChange={(event) => setRenameValue(event.target.value)}
              disabled={renameBusy}
              autoFocus
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRenameTarget(null)} disabled={renameBusy}>
              Cancel
            </Button>
            <Button onClick={() => void renameGroup()} disabled={renameBusy}>
              {renameBusy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              Save changes
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !deleteBusy) setDeleteTarget(null);
        }}
      >
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Delete group?</DialogTitle>
            <DialogDescription>
              “{deleteTarget?.name}” is removed from the demo dataset. The live API has no delete
              route for groups yet.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setDeleteTarget(null)} disabled={deleteBusy}>
              Cancel
            </Button>
            <Button variant="destructive" onClick={() => void deleteGroup()} disabled={deleteBusy}>
              {deleteBusy ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Trash2 className="size-4" aria-hidden="true" />
              )}
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Sheet open={membersGroup !== null} onOpenChange={(open) => !open && closeMembers()}>
        <SheetContent side="right" className="w-full sm:max-w-lg">
          {membersGroup ? (
            <>
              <SheetHeader>
                <SheetTitle>{membersGroup.name}</SheetTitle>
                <SheetDescription>
                  Add customers by searching their name or email, or remove one by id.
                </SheetDescription>
              </SheetHeader>
              <div className="flex-1 space-y-5 overflow-y-auto p-6">
                <div className="space-y-2">
                  <Label htmlFor="member-search">Add a customer</Label>
                  <div className="relative">
                    <Search
                      className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-faint"
                      aria-hidden="true"
                    />
                    <Input
                      id="member-search"
                      value={memberQuery}
                      onChange={(event) => setMemberQuery(event.target.value)}
                      placeholder="Search by name or email"
                      className="pl-9"
                    />
                  </div>
                  {memberSearching ? (
                    <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
                      <Loader2 className="size-3 animate-spin" aria-hidden="true" />
                      Searching customers…
                    </p>
                  ) : null}
                  <ul className="divide-y divide-border overflow-hidden rounded-lg border border-border">
                    {suggestedMembers.slice(0, 8).map((candidate) => {
                      const alreadyAdded = activeSessionMembers.some(
                        (entry) => entry.id === candidate.id,
                      );
                      return (
                        <li
                          key={candidate.id}
                          className="flex items-center gap-3 bg-background px-3 py-2"
                        >
                          <Avatar className="size-7">
                            <AvatarFallback className="text-[9px]">
                              {initials(candidate.name)}
                            </AvatarFallback>
                          </Avatar>
                          <div className="min-w-0 flex-1">
                            <p className="truncate text-[12px] text-foreground">{candidate.name}</p>
                            <p className="truncate text-[10.5px] text-faint">{candidate.email}</p>
                          </div>
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={alreadyAdded || memberBusyId !== null || !writable}
                            onClick={() => void addMember(candidate)}
                          >
                            {memberBusyId === candidate.id ? (
                              <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
                            ) : (
                              <UserPlus className="size-3.5" aria-hidden="true" />
                            )}
                            {alreadyAdded ? "Added" : "Add"}
                          </Button>
                        </li>
                      );
                    })}
                    {suggestedMembers.length === 0 ? (
                      <li className="bg-background px-3 py-4 text-[11.5px] text-muted-foreground">
                        {memberQuery.trim().length === 0
                          ? "Type a name or email to find customers."
                          : "No customers match that search."}
                      </li>
                    ) : null}
                  </ul>
                  <p className="text-[10.5px] text-faint">
                    The API does not expose a group member listing; this list shows the results of
                    the current search.
                  </p>
                </div>

                {activeSessionMembers.length > 0 ? (
                  <div className="space-y-2">
                    <Label>Added this session</Label>
                    <ul className="flex flex-wrap gap-2">
                      {activeSessionMembers.map((member) => (
                        <li
                          key={member.id}
                          className="inline-flex items-center gap-1.5 rounded-full border border-border bg-secondary px-2.5 py-1 text-[11px] text-foreground"
                        >
                          {member.name}
                          <button
                            type="button"
                            className="cursor-pointer text-faint hover:text-destructive"
                            aria-label={`Remove ${member.name}`}
                            disabled={removeBusy || !writable}
                            onClick={() => void removeMemberById(member.id, member.name)}
                          >
                            <X className="size-3" />
                          </button>
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}

                <div className="space-y-2 border-t border-border pt-4">
                  <Label htmlFor="member-remove-id">Remove a customer by id</Label>
                  <div className="flex gap-2">
                    <Input
                      id="member-remove-id"
                      value={removeId}
                      onChange={(event) => setRemoveId(event.target.value)}
                      placeholder="cust_..."
                      className="font-mono"
                      disabled={removeBusy}
                    />
                    <Button
                      variant="outline"
                      className="shrink-0"
                      disabled={removeBusy || removeId.trim().length === 0 || !writable}
                      onClick={() => void removeMemberById(removeId)}
                    >
                      {removeBusy ? (
                        <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                      ) : (
                        <UserMinus className="size-4" aria-hidden="true" />
                      )}
                      Remove
                    </Button>
                  </div>
                  <p className="text-[10.5px] text-faint">
                    Removing a customer that is not a member has no effect.
                  </p>
                </div>
              </div>
            </>
          ) : null}
        </SheetContent>
      </Sheet>
    </div>
  );
}

function CreateGroupDialog({
  open,
  onOpenChange,
  name,
  error,
  busy,
  onNameChange,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  name: string;
  error: string | null;
  busy: boolean;
  onNameChange: (value: string) => void;
  onSubmit: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <form
          className="space-y-5"
          noValidate
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>New customer group</DialogTitle>
            <DialogDescription>
              Names are unique per store. Membership is managed after the group exists.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="group-name">Name</Label>
            <Input
              id="group-name"
              value={name}
              onChange={(event) => onNameChange(event.target.value)}
              placeholder="VIP customers"
              autoComplete="off"
              autoFocus
              disabled={busy}
              aria-invalid={error !== null}
            />
            {error ? (
              <p className="text-[11.5px] text-destructive" role="alert">
                {error}
              </p>
            ) : null}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
              disabled={busy}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" aria-hidden="true" /> : null}
              Create group
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
