const { expect, test } = require("@playwright/test");
const { expectPageContentMounted, navigateAndWait, watchPageErrors } = require("../helpers");

test.describe("Agents settings page", () => {
	test("settings/agents loads and shows heading", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await expect(page).toHaveURL(/\/settings\/agents$/);
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("main agent card is shown with Default badge", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const mainCard = page.locator(".backend-card").filter({ hasText: "Default" });
		await expect(mainCard).toBeVisible();

		// Main agent should have an "Identity Settings" button, not Edit/Delete
		await expect(mainCard.getByRole("button", { name: "Identity Settings", exact: true })).toBeVisible();
		await expect(mainCard.getByRole("button", { name: "Edit", exact: true })).toHaveCount(0);
		await expect(mainCard.getByRole("button", { name: "Delete", exact: true })).toHaveCount(0);

		expect(pageErrors).toEqual([]);
	});

	test("New Agent button opens create form", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const newBtn = page.getByRole("button", { name: "New Agent", exact: true });
		await expect(newBtn).toBeVisible();
		await newBtn.click();

		// Form should be visible with ID, Name, and Create/Cancel buttons
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();
		await expect(page.getByPlaceholder("e.g. writer, coder, researcher")).toBeVisible();
		await expect(page.getByPlaceholder("Creative Writer")).toBeVisible();
		await expect(page.getByRole("button", { name: "Create", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("create form Cancel button returns to list", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		await page.getByRole("button", { name: "Cancel", exact: true }).click();

		// Should be back to the agent list with heading and New Agent button
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible();
		await expect(page.getByRole("button", { name: "New Agent", exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("create, edit, and delete an agent", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		// Create a new agent
		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		const idInput = page.getByPlaceholder("e.g. writer, coder, researcher");
		const nameInput = page.getByPlaceholder("Creative Writer");
		await idInput.fill("e2e-test-agent");
		await nameInput.fill("E2E Test Agent");
		await page.getByRole("button", { name: "Create", exact: true }).click();

		// Should return to the list and show the new agent
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		const agentCard = page.locator(".backend-card").filter({ hasText: "E2E Test Agent" });
		await expect(agentCard).toBeVisible();
		await expect(agentCard.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
		await expect(agentCard.getByRole("button", { name: "Delete", exact: true })).toBeVisible();

		// Edit the agent
		await agentCard.getByRole("button", { name: "Edit", exact: true }).click();
		await expect(page.getByText("Edit E2E Test Agent", { exact: true })).toBeVisible();

		const editNameInput = page.getByPlaceholder("Creative Writer");
		await editNameInput.fill("E2E Renamed Agent");
		await page.getByRole("button", { name: "Save", exact: true }).click();

		// Should return to the list with updated name
		await expect(page.getByRole("heading", { name: "Agents", exact: true })).toBeVisible({ timeout: 10_000 });
		const renamedCard = page.locator(".backend-card").filter({ hasText: "E2E Renamed Agent" });
		await expect(renamedCard).toBeVisible();

		// Delete the agent
		page.on("dialog", (dialog) => dialog.accept());
		await renamedCard.getByRole("button", { name: "Delete", exact: true }).click();

		// Agent should be removed from the list
		await expect(renamedCard).toHaveCount(0, { timeout: 10_000 });

		expect(pageErrors).toEqual([]);
	});

	test("create form validates required fields", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		await page.getByRole("button", { name: "New Agent", exact: true }).click();
		await expect(page.getByText("Create Agent", { exact: true })).toBeVisible();

		// Submit with empty fields
		await page.getByRole("button", { name: "Create", exact: true }).click();
		await expect(page.getByText("Name is required.", { exact: true })).toBeVisible();

		// Fill name but not ID
		await page.getByPlaceholder("Creative Writer").fill("Test");
		await page.getByRole("button", { name: "Create", exact: true }).click();
		await expect(page.getByText("ID is required.", { exact: true })).toBeVisible();

		expect(pageErrors).toEqual([]);
	});

	test("Identity Settings button on main agent navigates to identity page", async ({ page }) => {
		const pageErrors = watchPageErrors(page);
		await navigateAndWait(page, "/settings/agents");

		const mainCard = page.locator(".backend-card").filter({ hasText: "Default" });
		await mainCard.getByRole("button", { name: "Identity Settings", exact: true }).click();

		await expect(page).toHaveURL(/\/settings\/identity$/);
		await expectPageContentMounted(page);

		expect(pageErrors).toEqual([]);
	});
});
